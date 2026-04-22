//! Rich-text widget with mouse-drag selection and Cmd+C copy.
//!
//! Mirrors `iced::widget::rich_text` and its internal `Rich` widget
//! (at `iced_widget-0.13.4/src/text/rich.rs`) and layers selection on
//! top: click-drag builds a byte-range selection over the concatenated
//! span text, Cmd/Ctrl+C copies that range to the system clipboard.
//!
//! Rendering note: selection highlight is character-precise — for each
//! visual line of each overlapping span we binary-search `hit_test` to
//! find the pixel boundaries of the selected byte range and paint only
//! that sub-rect. Clipboard text is sliced character-accurately to match.

use iced::advanced::clipboard::{self, Clipboard};
use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::renderer;
use iced::advanced::text::{Hit, Paragraph, Span};
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{self, Layout, Shell, Widget};
use iced::widget::text::{self, Catalog, LineHeight, Shaping, Style, StyleFn, Wrapping};
use iced::{
    alignment, event, keyboard, window, Color, Element, Event, Length, Pixels,
    Point, Rectangle, Size, Vector,
};
use std::ops::Range;
use std::time::{Duration, Instant};

const MULTI_CLICK_WINDOW: Duration = Duration::from_millis(500);
/// Max distance (in bytes) a second click can land from the first and still
/// count as a double/triple-click. Bytes, not pixels — `hit_test` returns
/// byte offsets and this keeps the comparison cheap.
const MULTI_CLICK_BYTE_TOLERANCE: usize = 10;

/// A rich-text widget that supports mouse selection and copy-to-clipboard.
#[allow(missing_debug_implementations)]
pub struct SelectableRichText<'a, Link, Theme = iced::Theme, Renderer = iced::Renderer>
where
    Link: Clone + 'static,
    Theme: Catalog,
    Renderer: advanced::text::Renderer,
{
    spans: Box<dyn AsRef<[Span<'a, Link, Renderer::Font>]> + 'a>,
    size: Option<Pixels>,
    line_height: LineHeight,
    width: Length,
    height: Length,
    font: Option<Renderer::Font>,
    align_x: alignment::Horizontal,
    align_y: alignment::Vertical,
    wrapping: Wrapping,
    class: Theme::Class<'a>,
    selection_color: Color,
}

impl<'a, Link, Theme, Renderer> SelectableRichText<'a, Link, Theme, Renderer>
where
    Link: Clone + 'static,
    Theme: Catalog,
    Renderer: advanced::text::Renderer,
    Renderer::Font: 'a,
{
    pub fn new() -> Self {
        Self {
            spans: Box::new([]),
            size: None,
            line_height: LineHeight::default(),
            width: Length::Shrink,
            height: Length::Shrink,
            font: None,
            align_x: alignment::Horizontal::Left,
            align_y: alignment::Vertical::Top,
            wrapping: Wrapping::default(),
            class: Theme::default(),
            // Soft tint of the configured accent color — picks up the
            // user's accent choice from Settings instead of a fixed hue.
            selection_color: Color {
                a: 0.30,
                ..crate::ui::theme::accent()
            },
        }
    }

    pub fn with_spans(
        spans: impl AsRef<[Span<'a, Link, Renderer::Font>]> + 'a,
    ) -> Self {
        Self {
            spans: Box::new(spans),
            ..Self::new()
        }
    }

    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = Some(size.into());
        self
    }

    pub fn line_height(mut self, line_height: impl Into<LineHeight>) -> Self {
        self.line_height = line_height.into();
        self
    }

    pub fn font(mut self, font: impl Into<Renderer::Font>) -> Self {
        self.font = Some(font.into());
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    #[allow(dead_code)]
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    pub fn selection_color(mut self, color: Color) -> Self {
        self.selection_color = color;
        self
    }

    #[allow(dead_code)]
    pub fn wrapping(mut self, wrapping: Wrapping) -> Self {
        self.wrapping = wrapping;
        self
    }

    #[must_use]
    pub fn style(mut self, style: impl Fn(&Theme) -> Style + 'a) -> Self
    where
        Theme::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        self.class = (Box::new(style) as StyleFn<'a, Theme>).into();
        self
    }

    pub fn color(self, color: impl Into<Color>) -> Self
    where
        Theme::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        let color = Some(color.into());
        self.style(move |_theme| Style { color })
    }
}

impl<'a, Link, Theme, Renderer> Default for SelectableRichText<'a, Link, Theme, Renderer>
where
    Link: Clone + 'a,
    Theme: Catalog,
    Renderer: advanced::text::Renderer,
    Renderer::Font: 'a,
{
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
struct DragState {
    anchor: usize,
    cursor: usize,
}

#[derive(Debug, Clone, Copy)]
struct LastClick {
    at: Instant,
    byte: usize,
    count: u8,
}

struct State<Link, P: Paragraph> {
    spans: Vec<Span<'static, Link, P::Font>>,
    span_pressed: Option<usize>,
    paragraph: P,
    drag: Option<DragState>,
    selection: Option<Range<usize>>,
    last_click: Option<LastClick>,
    /// Tracked separately because `mouse::Event::ButtonPressed` doesn't
    /// carry modifier state in iced 0.13; we latch it on `ModifiersChanged`.
    modifiers: keyboard::Modifiers,
    /// True after a click landed inside our bounds and no click has
    /// landed elsewhere since. Used to gate Cmd+A to the "active"
    /// paragraph so every paragraph on screen doesn't select at once.
    focused: bool,
}

impl<'a, Link, Theme, Renderer> Widget<Link, Theme, Renderer>
    for SelectableRichText<'a, Link, Theme, Renderer>
where
    Link: Clone + 'static,
    Theme: Catalog,
    Renderer: advanced::text::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<Link, Renderer::Paragraph>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::<Link, _> {
            spans: Vec::new(),
            span_pressed: None,
            paragraph: Renderer::Paragraph::default(),
            drag: None,
            selection: None,
            last_click: None,
            modifiers: keyboard::Modifiers::default(),
            focused: false,
        })
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout(
            tree.state
                .downcast_mut::<State<Link, Renderer::Paragraph>>(),
            renderer,
            limits,
            self.width,
            self.height,
            self.spans.as_ref().as_ref(),
            self.line_height,
            self.size,
            self.font,
            self.align_x,
            self.align_y,
            self.wrapping,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree
            .state
            .downcast_ref::<State<Link, Renderer::Paragraph>>();

        let style = theme.style(&self.class);

        let hovered_span = cursor
            .position_in(layout.bounds())
            .and_then(|position| state.paragraph.hit_span(position));

        let translation = layout.position() - Point::ORIGIN;

        // Draw order, bottom to top:
        //   1. span.highlight backgrounds (e.g. inline-code dark bg)
        //   2. selection highlight (must sit ABOVE code bg or it's hidden)
        //   3. underlines, strikethroughs, hovered-link underlines
        //   4. text
        for (index, span) in self.spans.as_ref().as_ref().iter().enumerate() {
            if let Some(highlight) = span.highlight {
                let regions = state.paragraph.span_bounds(index);
                for bounds in &regions {
                    let bounds = Rectangle::new(
                        bounds.position()
                            - Vector::new(
                                span.padding.left,
                                span.padding.top,
                            ),
                        bounds.size()
                            + Size::new(
                                span.padding.horizontal(),
                                span.padding.vertical(),
                            ),
                    );

                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: bounds + translation,
                            border: highlight.border,
                            ..Default::default()
                        },
                        highlight.background,
                    );
                }
            }
        }

        if let Some(range) = state.selection.as_ref() {
            let spans_ref: &[Span<'a, Link, Renderer::Font>] =
                self.spans.as_ref().as_ref();
            // hit_test returns cursor.index which is BufferLine-relative;
            // convert to absolute using the BufferLine start derived from
            // the concatenated span text. A single span may span multiple
            // BufferLines (embedded `\n`, hard breaks), so we track the
            // current BufferLine per-rect as we walk in y-order rather
            // than per-span.
            let concat = concat_text(spans_ref);
            let line_starts_abs: Vec<usize> = {
                let mut v = vec![0usize];
                for (pos, _) in concat.match_indices('\n') {
                    v.push(pos + 1);
                }
                v
            };
            let mut current_bl_idx = 0usize;
            let mut last_y: Option<f32> = None;
            let mut cursor_byte: usize = 0;
            for (i, span) in spans_ref.iter().enumerate() {
                let span_start = cursor_byte;
                let span_len = span.text.as_ref().len();
                let span_end = cursor_byte + span_len;
                cursor_byte = span_end;

                let overlap_start = range.start.max(span_start);
                let overlap_end = range.end.min(span_end);

                for rect in state.paragraph.span_bounds(i) {
                    let y_mid = rect.y + rect.height * 0.5;
                    let probe_left = state
                        .paragraph
                        .hit_test(Point::new(rect.x + 0.25, y_mid))
                        .map(|h| match h {
                            Hit::CharOffset(b) => b,
                        })
                        .unwrap_or(0);
                    if let Some(ly) = last_y {
                        if rect.y > ly + 0.5 && probe_left == 0 {
                            let line_h = rect.height.max(1.0);
                            let gap_lines = ((rect.y - ly - line_h) / line_h)
                                .max(0.0)
                                .round()
                                as usize;
                            current_bl_idx = (current_bl_idx + 1 + gap_lines)
                                .min(line_starts_abs.len().saturating_sub(1));
                        }
                    }
                    last_y = Some(rect.y);

                    if overlap_start >= overlap_end {
                        continue;
                    }

                    let bl_start = line_starts_abs[current_bl_idx];

                    if let Some(sub) = sub_rect_for_range(
                        &state.paragraph,
                        rect,
                        span_start,
                        span_end,
                        overlap_start,
                        overlap_end,
                        bl_start,
                    ) {
                        renderer.fill_quad(
                            renderer::Quad {
                                bounds: sub + translation,
                                ..Default::default()
                            },
                            self.selection_color,
                        );
                    }
                }
            }
        }

        for (index, span) in self.spans.as_ref().as_ref().iter().enumerate() {
            let is_hovered_link =
                span.link.is_some() && Some(index) == hovered_span;

            if !(span.underline || span.strikethrough || is_hovered_link) {
                continue;
            }

            let regions = state.paragraph.span_bounds(index);
            let size = span
                .size
                .or(self.size)
                .unwrap_or(renderer.default_size());

            let line_height = span
                .line_height
                .unwrap_or(self.line_height)
                .to_absolute(size);

            let color = span
                .color
                .or(style.color)
                .unwrap_or(defaults.text_color);

            let baseline = translation
                + Vector::new(0.0, size.0 + (line_height.0 - size.0) / 2.0);

            if span.underline || is_hovered_link {
                for bounds in &regions {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle::new(
                                bounds.position() + baseline
                                    - Vector::new(0.0, size.0 * 0.08),
                                Size::new(bounds.width, 1.0),
                            ),
                            ..Default::default()
                        },
                        color,
                    );
                }
            }

            if span.strikethrough {
                for bounds in &regions {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle::new(
                                bounds.position() + baseline
                                    - Vector::new(0.0, size.0 / 2.0),
                                Size::new(bounds.width, 1.0),
                            ),
                            ..Default::default()
                        },
                        color,
                    );
                }
            }
        }

        text::draw(
            renderer,
            defaults,
            layout,
            &state.paragraph,
            style,
            viewport,
        );
    }

    fn on_event(
        &mut self,
        tree: &mut Tree,
        event: Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Link>,
        _viewport: &Rectangle,
    ) -> event::Status {
        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
                let state = tree
                    .state
                    .downcast_mut::<State<Link, Renderer::Paragraph>>();
                state.modifiers = m;
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(position) = cursor.position_in(layout.bounds()) {
                    let state = tree
                        .state
                        .downcast_mut::<State<Link, Renderer::Paragraph>>();

                    let byte = hit_test_absolute(
                        &state.paragraph,
                        self.spans.as_ref().as_ref(),
                        position,
                    )
                    .unwrap_or(0);

                    // Multi-click tracking: advance count on rapid clicks
                    // landing near the same byte; otherwise reset to 1.
                    let now = Instant::now();
                    let count = match state.last_click {
                        Some(lc)
                            if now.duration_since(lc.at) < MULTI_CLICK_WINDOW
                                && lc.byte.abs_diff(byte)
                                    <= MULTI_CLICK_BYTE_TOLERANCE =>
                        {
                            lc.count.saturating_add(1).min(3)
                        }
                        _ => 1,
                    };
                    state.last_click = Some(LastClick { at: now, byte, count });
                    state.focused = true;
                    state.span_pressed = state.paragraph.hit_span(position);

                    let total_len =
                        total_bytes(self.spans.as_ref().as_ref());
                    let shift = state.modifiers.shift();

                    if count >= 3 {
                        // Triple-click: select the entire widget's text.
                        state.drag = None;
                        state.selection = if total_len > 0 {
                            Some(0..total_len)
                        } else {
                            None
                        };
                    } else if count == 2 {
                        // Double-click: snap to word around the click.
                        let concat = concat_text(self.spans.as_ref().as_ref());
                        let word = word_range_at(&concat, byte);
                        if word.start < word.end {
                            state.selection = Some(word);
                            state.drag = None;
                        } else {
                            // Clicked on a non-word char (space/punct):
                            // fall back to drag-selection start.
                            state.selection = None;
                            state.drag = Some(DragState {
                                anchor: byte,
                                cursor: byte,
                            });
                        }
                    } else if shift {
                        // Shift+click: extend selection — keep the edge of
                        // the current selection farther from the click as
                        // the anchor, move the other edge to the click.
                        let anchor = match state.selection.as_ref() {
                            Some(range) => {
                                let dist_start = byte.abs_diff(range.start);
                                let dist_end = byte.abs_diff(range.end);
                                if dist_start >= dist_end {
                                    range.start
                                } else {
                                    range.end
                                }
                            }
                            None => state
                                .drag
                                .map(|d| d.anchor)
                                .unwrap_or(byte),
                        };
                        let (lo, hi) = normalize(anchor, byte);
                        state.selection =
                            if lo < hi { Some(lo..hi) } else { None };
                        state.drag = Some(DragState { anchor, cursor: byte });
                    } else {
                        // Single click: clear selection, start drag.
                        state.drag = Some(DragState {
                            anchor: byte,
                            cursor: byte,
                        });
                        state.selection = None;
                    }

                    shell.request_redraw(window::RedrawRequest::NextFrame);
                    return event::Status::Captured;
                } else {
                    // Click outside our bounds: drop focus + selection so
                    // nothing lingers on a widget the user has moved on from.
                    let state = tree
                        .state
                        .downcast_mut::<State<Link, Renderer::Paragraph>>();
                    let mut dirty = false;
                    if state.selection.is_some() {
                        state.selection = None;
                        dirty = true;
                    }
                    if state.focused {
                        state.focused = false;
                        dirty = true;
                    }
                    // Reset the multi-click chain so a click here followed
                    // by a click back on us is treated as a fresh single.
                    state.last_click = None;
                    if dirty {
                        shell.request_redraw(window::RedrawRequest::NextFrame);
                    }
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let state = tree
                    .state
                    .downcast_mut::<State<Link, Renderer::Paragraph>>();

                if let Some(drag) = state.drag.as_mut() {
                    // Only update while cursor is inside our bounds —
                    // outside, selection keeps its last value.
                    if let Some(position) = cursor.position_in(layout.bounds())
                    {
                        if let Some(byte) = hit_test_absolute(
                            &state.paragraph,
                            self.spans.as_ref().as_ref(),
                            position,
                        ) {
                            if byte != drag.cursor {
                                drag.cursor = byte;
                                let (lo, hi) =
                                    normalize(drag.anchor, drag.cursor);
                                state.selection = if lo == hi {
                                    None
                                } else {
                                    Some(lo..hi)
                                };
                                shell.request_redraw(
                                    window::RedrawRequest::NextFrame,
                                );
                            }
                        }
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let state = tree
                    .state
                    .downcast_mut::<State<Link, Renderer::Paragraph>>();

                let span_pressed = state.span_pressed.take();
                let drag = state.drag.take();

                // Plain click (no drag movement) → fire link, same as Rich.
                let moved = drag.map(|d| d.anchor != d.cursor).unwrap_or(false);

                if !moved {
                    if let Some(span_pressed) = span_pressed {
                        if let Some(position) =
                            cursor.position_in(layout.bounds())
                        {
                            if let Some(span) =
                                state.paragraph.hit_span(position)
                            {
                                if span == span_pressed {
                                    if let Some(link) = self
                                        .spans
                                        .as_ref()
                                        .as_ref()
                                        .get(span)
                                        .and_then(|s| s.link.clone())
                                    {
                                        shell.publish(link);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                ref key,
                modifiers,
                ..
            }) => {
                if !modifiers.command() {
                    return event::Status::Ignored;
                }

                let key_ref = key.as_ref();
                let is_copy = matches!(
                    key_ref,
                    keyboard::Key::Character("c")
                        | keyboard::Key::Character("C")
                );
                let is_select_all = matches!(
                    key_ref,
                    keyboard::Key::Character("a")
                        | keyboard::Key::Character("A")
                );

                if is_copy {
                    let state = tree
                        .state
                        .downcast_ref::<State<Link, Renderer::Paragraph>>();
                    if let Some(range) = state.selection.as_ref() {
                        let text =
                            extract(self.spans.as_ref().as_ref(), range);
                        if !text.is_empty() {
                            clipboard
                                .write(clipboard::Kind::Standard, text);
                            return event::Status::Captured;
                        }
                    }
                } else if is_select_all {
                    let state = tree
                        .state
                        .downcast_mut::<State<Link, Renderer::Paragraph>>();
                    // Only the "active" paragraph responds — otherwise
                    // every SelectableRichText on screen would select at
                    // once and we'd fight the launcher text_input for the
                    // shortcut too aggressively.
                    if state.focused {
                        let total =
                            total_bytes(self.spans.as_ref().as_ref());
                        if total > 0 {
                            state.selection = Some(0..total);
                            shell.request_redraw(
                                window::RedrawRequest::NextFrame,
                            );
                            return event::Status::Captured;
                        }
                    }
                }
            }
            _ => {}
        }

        event::Status::Ignored
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if let Some(position) = cursor.position_in(layout.bounds()) {
            let state = tree
                .state
                .downcast_ref::<State<Link, Renderer::Paragraph>>();

            if let Some(span) = state
                .paragraph
                .hit_span(position)
                .and_then(|span| self.spans.as_ref().as_ref().get(span))
            {
                if span.link.is_some() {
                    return mouse::Interaction::Pointer;
                }
            }

            return mouse::Interaction::Text;
        }

        mouse::Interaction::None
    }
}

fn normalize(a: usize, b: usize) -> (usize, usize) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Hit-test that returns an absolute byte offset in the concatenated span
/// text. Iced 0.13's `Paragraph::hit_test` exposes only `cursor.index`
/// (byte within a BufferLine), so a paragraph that embeds `\n` resets the
/// returned index to 0 on every line. Walk the span rects in visual
/// y-order, tracking the BufferLine index (new when `probe_left==0` at a
/// new y; vertical gaps account for empty BufferLines that carry no rect),
/// and use the BufferLine start of the rect containing the point.
fn hit_test_absolute<Link, Font, P>(
    paragraph: &P,
    spans: &[Span<'_, Link, Font>],
    point: Point,
) -> Option<usize>
where
    P: Paragraph,
{
    let Hit::CharOffset(local) = paragraph.hit_test(point)?;
    let concat = concat_text(spans);
    let line_starts_abs: Vec<usize> = {
        let mut v = vec![0usize];
        for (pos, _) in concat.match_indices('\n') {
            v.push(pos + 1);
        }
        v
    };

    let mut current_bl_idx = 0usize;
    let mut last_y: Option<f32> = None;
    let mut best: Option<(f32, usize)> = None;

    for (i, _span) in spans.iter().enumerate() {
        for rect in paragraph.span_bounds(i) {
            let y_mid = rect.y + rect.height * 0.5;
            let probe_left = paragraph
                .hit_test(Point::new(rect.x + 0.25, y_mid))
                .map(|h| match h {
                    Hit::CharOffset(b) => b,
                })
                .unwrap_or(0);
            if let Some(ly) = last_y {
                if rect.y > ly + 0.5 && probe_left == 0 {
                    let line_h = rect.height.max(1.0);
                    let gap_lines =
                        ((rect.y - ly - line_h) / line_h).max(0.0).round() as usize;
                    current_bl_idx = (current_bl_idx + 1 + gap_lines)
                        .min(line_starts_abs.len().saturating_sub(1));
                }
            }
            last_y = Some(rect.y);

            if point.y >= rect.y
                && point.y <= rect.y + rect.height
                && point.x >= rect.x
                && point.x <= rect.x + rect.width
            {
                return Some(
                    (line_starts_abs[current_bl_idx] + local).min(concat.len()),
                );
            }
            let dy = if point.y < rect.y {
                rect.y - point.y
            } else if point.y > rect.y + rect.height {
                point.y - (rect.y + rect.height)
            } else {
                0.0
            };
            if best.is_none_or(|(d, _)| dy < d) {
                best = Some((dy, line_starts_abs[current_bl_idx]));
            }
        }
    }
    let bl_start = best.map(|(_, s)| s).unwrap_or(0);
    Some((bl_start + local).min(concat.len()))
}

/// Character-precise selection rectangle for a single `span_bounds` line.
/// See `selectable_bubble::sub_rect_for_range` — same logic. Duplicated
/// because the two widgets carry different `Paragraph` generics and the
/// helper can't be shared without extra plumbing.
fn sub_rect_for_range<P: Paragraph>(
    paragraph: &P,
    rect: Rectangle,
    span_start: usize,
    span_end: usize,
    ovr_start: usize,
    ovr_end: usize,
    bl_start: usize,
) -> Option<Rectangle> {
    let y = rect.y + rect.height * 0.5;
    let x_right = rect.x + rect.width;

    let (line_lo, line_hi) =
        line_byte_range(paragraph, rect, span_start, span_end, bl_start)?;
    let clip_start = ovr_start.max(line_lo);
    let clip_end = ovr_end.min(line_hi);
    if clip_start >= clip_end {
        return None;
    }

    let x_start = if clip_start <= line_lo {
        rect.x
    } else {
        x_for_byte(paragraph, clip_start, y, rect.x, x_right, bl_start)
    };
    let x_end = if clip_end >= line_hi {
        x_right
    } else {
        x_for_byte(paragraph, clip_end, y, rect.x, x_right, bl_start)
    };

    if x_end <= x_start {
        return None;
    }
    Some(Rectangle::new(
        Point::new(x_start, rect.y),
        Size::new(x_end - x_start, rect.height),
    ))
}

fn line_byte_range<P: Paragraph>(
    paragraph: &P,
    rect: Rectangle,
    span_start: usize,
    span_end: usize,
    bl_start: usize,
) -> Option<(usize, usize)> {
    let y = rect.y + rect.height * 0.5;
    let probe_left = paragraph
        .hit_test(Point::new(rect.x + 0.25, y))
        .map(|h| match h {
            Hit::CharOffset(b) => b + bl_start,
        });
    let probe_right = paragraph
        .hit_test(Point::new((rect.x + rect.width - 0.25).max(rect.x), y))
        .map(|h| match h {
            Hit::CharOffset(b) => b + bl_start,
        });
    let left = probe_left.unwrap_or(span_start).clamp(span_start, span_end);
    let right = probe_right.unwrap_or(span_end).clamp(span_start, span_end);
    let (lo, hi) = if left <= right { (left, right) } else { (right, left) };
    if lo >= hi { None } else { Some((lo, hi)) }
}

fn x_for_byte<P: Paragraph>(
    paragraph: &P,
    target: usize,
    y: f32,
    x_min: f32,
    x_max: f32,
    bl_start: usize,
) -> f32 {
    let target_local = target.saturating_sub(bl_start);
    let mut lo = x_min;
    let mut hi = x_max;
    for _ in 0..24 {
        if hi - lo <= 0.25 {
            break;
        }
        let mid = (lo + hi) * 0.5;
        match paragraph.hit_test(Point::new(mid, y)) {
            Some(Hit::CharOffset(b)) => {
                if b < target_local {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            None => lo = mid,
        }
    }
    (lo + hi) * 0.5
}

fn extract<Link, Font>(
    spans: &[Span<'_, Link, Font>],
    range: &Range<usize>,
) -> String {
    let mut out = String::new();
    let mut cursor: usize = 0;
    for span in spans {
        let text = span.text.as_ref();
        let len = text.len();
        let span_range = cursor..cursor + len;

        let overlap_start = range.start.max(span_range.start);
        let overlap_end = range.end.min(span_range.end);
        if overlap_start < overlap_end {
            let local_start = overlap_start - cursor;
            let local_end = overlap_end - cursor;
            // Clamp to valid char boundaries — defensive: hit_test should
            // return byte offsets that land on char boundaries, but floor
            // to the nearest valid boundary just in case.
            let start = floor_char_boundary(text, local_start);
            let end = floor_char_boundary(text, local_end);
            out.push_str(&text[start..end]);
        }

        cursor += len;
    }
    out
}

fn floor_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn total_bytes<Link, Font>(spans: &[Span<'_, Link, Font>]) -> usize {
    spans.iter().map(|s| s.text.as_ref().len()).sum()
}

fn concat_text<Link, Font>(spans: &[Span<'_, Link, Font>]) -> String {
    let mut out = String::with_capacity(total_bytes(spans));
    for span in spans {
        out.push_str(span.text.as_ref());
    }
    out
}

/// Word characters for double-click snap: alphanumeric + `_`. Matches the
/// "word" notion most editors use for identifiers. Hyphens are intentionally
/// excluded so double-clicking `cargo` in `cargo-build` gives you `cargo`.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Returns the byte range of the word containing `byte_offset`. Returns an
/// empty range when the click landed on whitespace or punctuation so the
/// caller can fall back to drag-selection.
fn word_range_at(text: &str, byte_offset: usize) -> Range<usize> {
    // Clamp to a valid char boundary. hit_test usually returns one, but be
    // defensive since we're about to slice strings with it.
    let mut idx = byte_offset.min(text.len());
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }

    // Peek the char at the click. If we're past the last char (click at
    // end-of-text), peek the previous char instead.
    let focus_char = text[idx..]
        .chars()
        .next()
        .or_else(|| text[..idx].chars().next_back());

    let Some(ch) = focus_char else {
        return idx..idx;
    };
    if !is_word_char(ch) {
        return idx..idx;
    }

    // Walk backward to find word start.
    let mut start = idx;
    while start > 0 {
        let Some((prev_idx, prev_ch)) = text[..start].char_indices().next_back() else {
            break;
        };
        if !is_word_char(prev_ch) {
            break;
        }
        start = prev_idx;
    }

    // Walk forward to find word end.
    let mut end = idx;
    while end < text.len() {
        let ch = match text[end..].chars().next() {
            Some(c) => c,
            None => break,
        };
        if !is_word_char(ch) {
            break;
        }
        end += ch.len_utf8();
    }

    start..end
}

#[allow(clippy::too_many_arguments)]
fn layout<Link, Renderer>(
    state: &mut State<Link, Renderer::Paragraph>,
    renderer: &Renderer,
    limits: &layout::Limits,
    width: Length,
    height: Length,
    spans: &[Span<'_, Link, Renderer::Font>],
    line_height: LineHeight,
    size: Option<Pixels>,
    font: Option<Renderer::Font>,
    horizontal_alignment: alignment::Horizontal,
    vertical_alignment: alignment::Vertical,
    wrapping: Wrapping,
) -> layout::Node
where
    Link: Clone,
    Renderer: advanced::text::Renderer,
{
    layout::sized(limits, width, height, |limits| {
        let bounds = limits.max();

        let size = size.unwrap_or_else(|| renderer.default_size());
        let font = font.unwrap_or_else(|| renderer.default_font());

        let text_with_spans = || advanced::text::Text {
            content: spans,
            bounds,
            size,
            line_height,
            font,
            horizontal_alignment,
            vertical_alignment,
            shaping: Shaping::Advanced,
            wrapping,
        };

        if state.spans != spans {
            state.paragraph =
                Renderer::Paragraph::with_spans(text_with_spans());
            state.spans =
                spans.iter().cloned().map(Span::to_static).collect();
            // Invalidate any stale selection — byte offsets may not map
            // meaningfully to the new text.
            state.selection = None;
            state.drag = None;
        } else {
            match state.paragraph.compare(advanced::text::Text {
                content: (),
                bounds,
                size,
                line_height,
                font,
                horizontal_alignment,
                vertical_alignment,
                shaping: Shaping::Advanced,
                wrapping,
            }) {
                advanced::text::Difference::None => {}
                advanced::text::Difference::Bounds => {
                    state.paragraph.resize(bounds);
                }
                advanced::text::Difference::Shape => {
                    state.paragraph =
                        Renderer::Paragraph::with_spans(text_with_spans());
                }
            }
        }

        state.paragraph.min_bounds()
    })
}

impl<'a, Link, Theme, Renderer> From<SelectableRichText<'a, Link, Theme, Renderer>>
    for Element<'a, Link, Theme, Renderer>
where
    Link: Clone + 'a,
    Theme: Catalog + 'a,
    Renderer: advanced::text::Renderer + 'a,
{
    fn from(
        text: SelectableRichText<'a, Link, Theme, Renderer>,
    ) -> Element<'a, Link, Theme, Renderer> {
        Element::new(text)
    }
}

/// Convenience constructor mirroring `iced::widget::rich_text`.
pub fn selectable_rich_text<'a, Link, Theme, Renderer>(
    spans: impl AsRef<[Span<'a, Link, Renderer::Font>]> + 'a,
) -> SelectableRichText<'a, Link, Theme, Renderer>
where
    Link: Clone + 'static,
    Theme: Catalog,
    Renderer: advanced::text::Renderer,
    Renderer::Font: 'a,
{
    SelectableRichText::with_spans(spans)
}
