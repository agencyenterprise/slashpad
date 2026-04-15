//! One-widget markdown renderer that owns a single text selection
//! spanning every block in a chat message (paragraphs, headings, code
//! blocks, list items).
//!
//! Why this exists: `selectable_markdown::view` produces a `Column` of
//! independent `SelectableRichText` children, so selection can't cross
//! block boundaries. This widget is a leaf — it does its own vertical
//! layout and owns one `Paragraph` per block internally. Mouse events are
//! handled at the container level, which lets drag naturally continue
//! from one block into the next.
//!
//! Scope trade-offs (v1):
//! - Code blocks word-wrap (no horizontal scroll). Trade-off for not
//!   needing to embed a scrollable child we can't hit-test across.
//! - Nested lists render inline with increasing indent — bullets attach
//!   to the first block emitted per list item. Deeply nested markdown
//!   won't look as polished as the stock `markdown::view` would render.
//! - Inline links still click on single-click-no-drag, like the rich text
//!   widget.

use iced::advanced::clipboard::{self, Clipboard};
use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::renderer;
use iced::advanced::text::{Hit, Paragraph, Span};
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{self, Layout, Shell, Widget};
use iced::font::Font;
use iced::widget::markdown::{self, HeadingLevel, Item, Settings, Style, Url};
use iced::widget::text::{Catalog, LineHeight, Shaping, Wrapping};
use iced::{
    alignment, event, keyboard, window, Background, Color, Element, Event,
    Length, Padding, Pixels, Point, Rectangle, Size, Vector,
};
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::theme;

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Left indent per nesting depth for list items.
const LIST_INDENT: f32 = 16.0;
/// Space between consecutive blocks.
const BLOCK_SPACING: f32 = 8.0;
/// Extra top padding for headings after the first block.
const HEADING_MARGIN_TOP: f32 = 6.0;
/// Padding around a code block's rendered text, inside the background.
const CODE_BLOCK_PADDING: Padding = Padding {
    top: 8.0,
    right: 10.0,
    bottom: 8.0,
    left: 10.0,
};
/// Bullet column width (reserved space for bullet/number on the left).
const BULLET_COL_WIDTH: f32 = 16.0;

const MULTI_CLICK_WINDOW: Duration = Duration::from_millis(500);
const MULTI_CLICK_BYTE_TOLERANCE: usize = 10;

// ---------------------------------------------------------------------------
// Internal block model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum BlockKind {
    Paragraph,
    Heading,
    CodeBlock,
    ListItem,
}

/// A block ready to be laid out. Holds the spans plus its visual knobs.
#[derive(Clone)]
struct Block {
    kind: BlockKind,
    spans: Arc<[Span<'static, Url>]>,
    size: Pixels,
    font: Option<Font>,
    bullet: Option<String>,
    indent_depth: usize,
    top_margin: f32,
    background: Option<Background>,
    /// Only used for code blocks — padding around the text inside the bg.
    inner_padding: Padding,
    /// Wrapping mode: WordOrGlyph for code, Word for everything else.
    wrapping: Wrapping,
}

fn heading_size(settings: &Settings, level: &HeadingLevel) -> Pixels {
    match level {
        HeadingLevel::H1 => settings.h1_size,
        HeadingLevel::H2 => settings.h2_size,
        HeadingLevel::H3 => settings.h3_size,
        HeadingLevel::H4 => settings.h4_size,
        HeadingLevel::H5 => settings.h5_size,
        HeadingLevel::H6 => settings.h6_size,
    }
}

fn flatten_items(
    items: &[Item],
    depth: usize,
    is_first_in_document: &mut bool,
    settings: &Settings,
    style: &Style,
    out: &mut Vec<Block>,
) {
    for item in items {
        match item {
            Item::Paragraph(text) => {
                out.push(Block {
                    kind: BlockKind::Paragraph,
                    spans: text.spans(*style),
                    size: settings.text_size,
                    font: None,
                    bullet: None,
                    indent_depth: depth,
                    top_margin: if *is_first_in_document {
                        0.0
                    } else {
                        BLOCK_SPACING
                    },
                    background: None,
                    inner_padding: Padding::ZERO,
                    wrapping: Wrapping::default(),
                });
                *is_first_in_document = false;
            }
            Item::Heading(level, text) => {
                out.push(Block {
                    kind: BlockKind::Heading,
                    spans: text.spans(*style),
                    size: heading_size(settings, level),
                    font: None,
                    bullet: None,
                    indent_depth: depth,
                    top_margin: if *is_first_in_document {
                        0.0
                    } else {
                        HEADING_MARGIN_TOP
                    },
                    background: None,
                    inner_padding: Padding::ZERO,
                    wrapping: Wrapping::default(),
                });
                *is_first_in_document = false;
            }
            Item::CodeBlock(text) => {
                out.push(Block {
                    kind: BlockKind::CodeBlock,
                    spans: text.spans(*style),
                    size: settings.code_size,
                    font: Some(Font::MONOSPACE),
                    bullet: None,
                    indent_depth: depth,
                    top_margin: if *is_first_in_document {
                        0.0
                    } else {
                        BLOCK_SPACING
                    },
                    background: Some(Background::Color(theme::SURFACE_2)),
                    inner_padding: CODE_BLOCK_PADDING,
                    wrapping: Wrapping::WordOrGlyph,
                });
                *is_first_in_document = false;
            }
            Item::List { start, items: sub_items } => {
                for (i, sub_item) in sub_items.iter().enumerate() {
                    let bullet = match start {
                        Some(s) => format!("{}.", s + i as u64),
                        None => "•".to_string(),
                    };
                    let before = out.len();
                    flatten_items(
                        sub_item,
                        depth + 1,
                        is_first_in_document,
                        settings,
                        style,
                        out,
                    );
                    if let Some(first) = out.get_mut(before) {
                        first.bullet = Some(bullet);
                        // List items downgrade Paragraph kind so later logic
                        // can know to reserve a bullet column.
                        if first.kind == BlockKind::Paragraph {
                            first.kind = BlockKind::ListItem;
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal widget state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct DragState {
    anchor_block: usize,
    anchor_byte: usize,
    cursor_block: usize,
    cursor_byte: usize,
}

/// Selection as a pair of (block_index, byte_offset) coordinates.
#[derive(Debug, Clone, Copy)]
struct Selection {
    start: (usize, usize),
    end: (usize, usize),
}

impl Selection {
    fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

#[derive(Debug, Clone, Copy)]
struct LastClick {
    at: Instant,
    block: usize,
    byte: usize,
    count: u8,
}

struct BlockLayout<P: Paragraph> {
    kind: BlockKind,
    spans: Arc<[Span<'static, Url>]>,
    paragraph: P,
    /// Y-offset of the block's text bounds within the widget.
    offset_y: f32,
    /// Height of the block's text bounds (not including margins).
    height: f32,
    /// Left X-offset of the block's text bounds within the widget.
    offset_x: f32,
    width: f32,
    /// Where to draw the background rect, if any.
    background_bounds: Option<Rectangle>,
    background: Option<Background>,
    bullet: Option<(Point, String, Pixels)>,
    /// Total bytes of concatenated span text (cached).
    total_bytes: usize,
}

struct State<P: Paragraph> {
    blocks: Vec<BlockLayout<P>>,
    selection: Option<Selection>,
    drag: Option<DragState>,
    modifiers: keyboard::Modifiers,
    focused: bool,
    span_pressed: Option<(usize, usize)>,
    last_click: Option<LastClick>,
    /// Fingerprint of the item list that produced `blocks`. We only rebuild
    /// when this changes (new message / re-render).
    flat_hash: u64,
    /// The max width we last laid out for — used to skip layout work when
    /// nothing changed.
    last_width: f32,
}

// ---------------------------------------------------------------------------
// Public widget
// ---------------------------------------------------------------------------

#[allow(missing_debug_implementations)]
pub struct SelectableBubble<'a, Theme = iced::Theme, Renderer = iced::Renderer>
where
    Theme: Catalog,
    Renderer: advanced::text::Renderer<Font = Font>,
{
    items: &'a [Item],
    settings: Settings,
    style: Style,
    width: Length,
    selection_color: Color,
    _phantom: std::marker::PhantomData<(Theme, Renderer)>,
}

impl<'a, Theme, Renderer> SelectableBubble<'a, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: advanced::text::Renderer<Font = Font>,
{
    pub fn new(items: &'a [Item], settings: Settings, style: Style) -> Self {
        Self {
            items,
            settings,
            style,
            width: Length::Fill,
            selection_color: Color::from_rgba(0.77, 0.63, 1.0, 0.30),
            _phantom: std::marker::PhantomData,
        }
    }
}

// Convenience constructor, matches the `selectable_markdown::view` shape so
// the caller site swaps cleanly.
pub fn view<'a, Theme, Renderer>(
    items: &'a [Item],
    settings: Settings,
    style: Style,
) -> Element<'a, Url, Theme, Renderer>
where
    Theme: Catalog + 'a,
    Renderer: advanced::text::Renderer<Font = Font> + 'a,
{
    Element::new(SelectableBubble::new(items, settings, style))
}

// ---------------------------------------------------------------------------
// Widget trait impl
// ---------------------------------------------------------------------------

impl<'a, Theme, Renderer> Widget<Url, Theme, Renderer>
    for SelectableBubble<'a, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: advanced::text::Renderer<Font = Font>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<Renderer::Paragraph>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::<Renderer::Paragraph> {
            blocks: Vec::new(),
            selection: None,
            drag: None,
            modifiers: keyboard::Modifiers::default(),
            focused: false,
            span_pressed: None,
            last_click: None,
            flat_hash: 0,
            last_width: 0.0,
        })
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: Length::Shrink,
        }
    }

    fn layout(
        &self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State<Renderer::Paragraph>>();
        layout_blocks(
            state,
            renderer,
            limits,
            self.width,
            self.items,
            &self.settings,
            &self.style,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State<Renderer::Paragraph>>();
        let translation = layout.position() - Point::ORIGIN;

        // 1. Block backgrounds (code blocks).
        for block in &state.blocks {
            if let (Some(bg), Some(rect)) =
                (block.background, block.background_bounds)
            {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: rect + translation,
                        border: iced::Border {
                            color: Color::TRANSPARENT,
                            width: 0.0,
                            radius: 6.0.into(),
                        },
                        ..Default::default()
                    },
                    bg,
                );
            }
        }

        // 2. Per-span highlight backgrounds (e.g. inline-code dark bg).
        // Must be drawn *before* the selection highlight or the selection
        // gets covered by the inline-code background.
        for block in &state.blocks {
            for (index, span) in block.spans.iter().enumerate() {
                if let Some(highlight) = span.highlight {
                    let block_translation =
                        translation + Vector::new(block.offset_x, block.offset_y);
                    for bounds in block.paragraph.span_bounds(index) {
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
                                bounds: bounds + block_translation,
                                border: highlight.border,
                                ..Default::default()
                            },
                            highlight.background,
                        );
                    }
                }
            }
        }

        // 3. Selection highlight (across blocks).
        if let Some(sel) = state.selection.as_ref().filter(|s| !s.is_empty()) {
            let (start, end) = order(sel.start, sel.end);
            for (bi, block) in state.blocks.iter().enumerate() {
                if bi < start.0 || bi > end.0 {
                    continue;
                }
                let byte_start = if bi == start.0 { start.1 } else { 0 };
                let byte_end = if bi == end.0 { end.1 } else { block.total_bytes };
                if byte_start >= byte_end {
                    continue;
                }
                let block_translation =
                    translation + Vector::new(block.offset_x, block.offset_y);
                paint_selection_in_block(
                    renderer,
                    &block.paragraph,
                    &block.spans,
                    byte_start..byte_end,
                    block_translation,
                    self.selection_color,
                );
            }
        }

        // 4. Bullets.
        for block in &state.blocks {
            if let Some((pos, bullet, size)) = &block.bullet {
                renderer.fill_text(
                    iced::advanced::text::Text {
                        content: bullet.clone(),
                        bounds: Size::new(BULLET_COL_WIDTH, size.0 * 1.4),
                        size: *size,
                        line_height: LineHeight::default(),
                        font: renderer.default_font(),
                        horizontal_alignment: alignment::Horizontal::Left,
                        vertical_alignment: alignment::Vertical::Top,
                        shaping: Shaping::Advanced,
                        wrapping: Wrapping::None,
                    },
                    *pos + translation,
                    defaults.text_color,
                    *viewport,
                );
            }
        }

        // 5. Decorations (underlines, strikethroughs, hovered-link underlines).
        let hovered = cursor
            .position_in(layout.bounds())
            .and_then(|pos| hit_span_in(state, pos));
        for (bi, block) in state.blocks.iter().enumerate() {
            let block_translation =
                translation + Vector::new(block.offset_x, block.offset_y);
            for (si, span) in block.spans.iter().enumerate() {
                let is_hovered_link =
                    span.link.is_some() && hovered == Some((bi, si));
                if !(span.underline || span.strikethrough || is_hovered_link) {
                    continue;
                }

                let size = span.size.unwrap_or(Pixels(13.0));
                let line_height = span
                    .line_height
                    .unwrap_or(LineHeight::default())
                    .to_absolute(size);
                let color = span.color.unwrap_or(defaults.text_color);
                let baseline = block_translation
                    + Vector::new(
                        0.0,
                        size.0 + (line_height.0 - size.0) / 2.0,
                    );

                for bounds in block.paragraph.span_bounds(si) {
                    if span.underline || is_hovered_link {
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
                    if span.strikethrough {
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
        }

        // 6. Text for each block's paragraph.
        for block in &state.blocks {
            let block_position = layout.position()
                + Vector::new(block.offset_x, block.offset_y);
            renderer.fill_paragraph(
                &block.paragraph,
                block_position,
                defaults.text_color,
                *viewport,
            );
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if let Some(pos) = cursor.position_in(layout.bounds()) {
            let state = tree
                .state
                .downcast_ref::<State<Renderer::Paragraph>>();
            if let Some((bi, si)) = hit_span_in(state, pos) {
                if let Some(block) = state.blocks.get(bi) {
                    if let Some(span) = block.spans.get(si) {
                        if span.link.is_some() {
                            return mouse::Interaction::Pointer;
                        }
                    }
                }
            }
            return mouse::Interaction::Text;
        }
        mouse::Interaction::None
    }

    fn on_event(
        &mut self,
        tree: &mut Tree,
        event: Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Url>,
        _viewport: &Rectangle,
    ) -> event::Status {
        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
                let state = tree
                    .state
                    .downcast_mut::<State<Renderer::Paragraph>>();
                state.modifiers = m;
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(layout.bounds()) {
                    let state = tree
                        .state
                        .downcast_mut::<State<Renderer::Paragraph>>();
                    let Some((bi, byte)) = hit_block_byte(state, pos) else {
                        return event::Status::Ignored;
                    };

                    // Multi-click tracking.
                    let now = Instant::now();
                    let count = match state.last_click {
                        Some(lc)
                            if lc.block == bi
                                && now.duration_since(lc.at) < MULTI_CLICK_WINDOW
                                && lc.byte.abs_diff(byte)
                                    <= MULTI_CLICK_BYTE_TOLERANCE =>
                        {
                            lc.count.saturating_add(1).min(3)
                        }
                        _ => 1,
                    };
                    state.last_click = Some(LastClick {
                        at: now,
                        block: bi,
                        byte,
                        count,
                    });
                    state.focused = true;
                    state.span_pressed = hit_span_in(state, pos);

                    let shift = state.modifiers.shift();

                    if count >= 3 {
                        // Triple-click: select the whole block.
                        if let Some(block) = state.blocks.get(bi) {
                            let total = block.total_bytes;
                            if total > 0 {
                                state.selection = Some(Selection {
                                    start: (bi, 0),
                                    end: (bi, total),
                                });
                            } else {
                                state.selection = None;
                            }
                            state.drag = None;
                        }
                    } else if count == 2 {
                        // Double-click: word snap within the clicked block.
                        if let Some(block) = state.blocks.get(bi) {
                            let concat = concat_spans(&block.spans);
                            let word = word_range_at(&concat, byte);
                            if word.start < word.end {
                                state.selection = Some(Selection {
                                    start: (bi, word.start),
                                    end: (bi, word.end),
                                });
                                state.drag = None;
                            } else {
                                state.selection = None;
                                state.drag = Some(DragState {
                                    anchor_block: bi,
                                    anchor_byte: byte,
                                    cursor_block: bi,
                                    cursor_byte: byte,
                                });
                            }
                        }
                    } else if shift {
                        // Shift+click: extend from far edge of current sel.
                        let anchor = match state.selection {
                            Some(sel) => {
                                let (first, second) = order(sel.start, sel.end);
                                let here = (bi, byte);
                                if block_byte_cmp(here, first).abs()
                                    >= block_byte_cmp(here, second).abs()
                                {
                                    first
                                } else {
                                    second
                                }
                            }
                            None => state
                                .drag
                                .map(|d| (d.anchor_block, d.anchor_byte))
                                .unwrap_or((bi, byte)),
                        };
                        let (s, e) = order(anchor, (bi, byte));
                        state.selection = if s != e {
                            Some(Selection { start: s, end: e })
                        } else {
                            None
                        };
                        state.drag = Some(DragState {
                            anchor_block: anchor.0,
                            anchor_byte: anchor.1,
                            cursor_block: bi,
                            cursor_byte: byte,
                        });
                    } else {
                        // Single click: start drag.
                        state.drag = Some(DragState {
                            anchor_block: bi,
                            anchor_byte: byte,
                            cursor_block: bi,
                            cursor_byte: byte,
                        });
                        state.selection = None;
                    }

                    shell.request_redraw(window::RedrawRequest::NextFrame);
                    return event::Status::Captured;
                } else {
                    // Click outside bubble: drop focus + selection.
                    let state = tree
                        .state
                        .downcast_mut::<State<Renderer::Paragraph>>();
                    let mut dirty = false;
                    if state.selection.is_some() {
                        state.selection = None;
                        dirty = true;
                    }
                    if state.focused {
                        state.focused = false;
                        dirty = true;
                    }
                    state.last_click = None;
                    if dirty {
                        shell.request_redraw(window::RedrawRequest::NextFrame);
                    }
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let state = tree
                    .state
                    .downcast_mut::<State<Renderer::Paragraph>>();
                // Copy out — DragState is Copy — to avoid holding a mutable
                // borrow across the immutable hit_block_byte call.
                if let Some(mut drag) = state.drag {
                    if let Some(pos) = cursor.position_in(layout.bounds()) {
                        if let Some((bi, byte)) = hit_block_byte(state, pos) {
                            if (bi, byte) != (drag.cursor_block, drag.cursor_byte)
                            {
                                drag.cursor_block = bi;
                                drag.cursor_byte = byte;
                                let (s, e) = order(
                                    (drag.anchor_block, drag.anchor_byte),
                                    (drag.cursor_block, drag.cursor_byte),
                                );
                                state.selection = if s != e {
                                    Some(Selection { start: s, end: e })
                                } else {
                                    None
                                };
                                state.drag = Some(drag);
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
                    .downcast_mut::<State<Renderer::Paragraph>>();
                let span_pressed = state.span_pressed.take();
                let drag = state.drag.take();

                let moved = drag
                    .map(|d| {
                        (d.anchor_block, d.anchor_byte)
                            != (d.cursor_block, d.cursor_byte)
                    })
                    .unwrap_or(false);

                if !moved {
                    if let Some((bi, si)) = span_pressed {
                        if let Some(pos) = cursor.position_in(layout.bounds()) {
                            if hit_span_in(state, pos) == Some((bi, si)) {
                                if let Some(link) = state
                                    .blocks
                                    .get(bi)
                                    .and_then(|b| b.spans.get(si))
                                    .and_then(|s| s.link.clone())
                                {
                                    shell.publish(link);
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
                        .downcast_ref::<State<Renderer::Paragraph>>();
                    if let Some(sel) =
                        state.selection.as_ref().filter(|s| !s.is_empty())
                    {
                        let text = extract_selection(&state.blocks, sel);
                        if !text.is_empty() {
                            clipboard
                                .write(clipboard::Kind::Standard, text);
                            return event::Status::Captured;
                        }
                    }
                } else if is_select_all {
                    let state = tree
                        .state
                        .downcast_mut::<State<Renderer::Paragraph>>();
                    if state.focused && !state.blocks.is_empty() {
                        let last = state.blocks.len() - 1;
                        let last_bytes = state.blocks[last].total_bytes;
                        if last_bytes > 0 || state.blocks.len() > 1 {
                            state.selection = Some(Selection {
                                start: (0, 0),
                                end: (last, last_bytes),
                            });
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
}

// ---------------------------------------------------------------------------
// Layout helper
// ---------------------------------------------------------------------------

fn layout_blocks<Renderer>(
    state: &mut State<Renderer::Paragraph>,
    renderer: &Renderer,
    limits: &layout::Limits,
    width: Length,
    items: &[Item],
    settings: &Settings,
    style: &Style,
) -> layout::Node
where
    Renderer: advanced::text::Renderer<Font = Font>,
{
    let max = limits.max();
    let available_width = match width {
        Length::Shrink => max.width.min(600.0),
        _ => max.width,
    };

    let mut flat: Vec<Block> = Vec::new();
    let mut first = true;
    flatten_items(items, 0, &mut first, settings, style, &mut flat);
    let new_hash = compute_hash(&flat);

    let rebuild = new_hash != state.flat_hash
        || (state.last_width - available_width).abs() > 0.5;

    if rebuild {
        state.blocks.clear();
        state.selection = None;
        state.drag = None;
        state.flat_hash = new_hash;
        state.last_width = available_width;

        let mut y = 0.0f32;
        for block in &flat {
            y += block.top_margin;

            let indent_x =
                (block.indent_depth as f32) * LIST_INDENT;
            let has_bullet = block.bullet.is_some();
            let bullet_reserve = if has_bullet { BULLET_COL_WIDTH } else { 0.0 };
            let text_x = indent_x + bullet_reserve + block.inner_padding.left;

            let inner_width = (available_width
                - text_x
                - block.inner_padding.right)
                .max(1.0);

            let spans: Arc<[Span<'static, Url>]> = block.spans.clone();
            let paragraph = Renderer::Paragraph::with_spans(
                advanced::text::Text {
                    content: &spans[..],
                    bounds: Size::new(inner_width, f32::INFINITY),
                    size: block.size,
                    line_height: LineHeight::default(),
                    font: block.font.unwrap_or_else(|| renderer.default_font()),
                    horizontal_alignment: alignment::Horizontal::Left,
                    vertical_alignment: alignment::Vertical::Top,
                    shaping: Shaping::Advanced,
                    wrapping: block.wrapping,
                },
            );

            let min = paragraph.min_bounds();
            let text_h = min.height;
            let text_w = min.width;

            let block_top_for_bg = y;
            let bg_bounds = if block.background.is_some() {
                Some(Rectangle::new(
                    Point::new(indent_x, block_top_for_bg),
                    Size::new(
                        available_width - indent_x,
                        text_h
                            + block.inner_padding.top
                            + block.inner_padding.bottom,
                    ),
                ))
            } else {
                None
            };

            let text_y = y + block.inner_padding.top;

            let bullet = block.bullet.as_ref().map(|b| {
                (
                    Point::new(indent_x, text_y),
                    b.clone(),
                    block.size,
                )
            });

            let total_bytes = span_bytes(&spans);

            state.blocks.push(BlockLayout {
                kind: block.kind,
                spans: spans.clone(),
                paragraph,
                offset_y: text_y,
                height: text_h,
                offset_x: text_x,
                width: text_w.max(inner_width),
                background_bounds: bg_bounds,
                background: block.background,
                bullet,
                total_bytes,
            });

            y += text_h
                + block.inner_padding.top
                + block.inner_padding.bottom;
        }

        let total_h = y;
        layout::Node::new(Size::new(available_width, total_h))
    } else {
        let total_h = state
            .blocks
            .last()
            .map(|b| {
                b.offset_y
                    + b.height
                    + b.background_bounds
                        .map(|r| r.y + r.height - (b.offset_y + b.height))
                        .unwrap_or(0.0)
            })
            .unwrap_or(0.0);
        layout::Node::new(Size::new(available_width, total_h))
    }
}

fn compute_hash(blocks: &[Block]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    for b in blocks {
        (b.kind as u8).hash(&mut h);
        b.size.0.to_bits().hash(&mut h);
        b.indent_depth.hash(&mut h);
        b.bullet.hash(&mut h);
        for span in b.spans.iter() {
            span.text.as_ref().hash(&mut h);
            span.size.map(|p| p.0.to_bits()).hash(&mut h);
            span.underline.hash(&mut h);
            span.strikethrough.hash(&mut h);
            span.link.as_ref().map(|u| u.as_str()).hash(&mut h);
        }
    }
    h.finish()
}

// ---------------------------------------------------------------------------
// Hit testing + selection helpers
// ---------------------------------------------------------------------------

fn hit_block_byte<P: Paragraph>(
    state: &State<P>,
    pos: Point,
) -> Option<(usize, usize)> {
    // Find the block whose vertical range contains pos.y. If pos is above
    // the first block, map to (0, 0). If below the last, map to (last, end).
    if state.blocks.is_empty() {
        return None;
    }
    if pos.y <= state.blocks[0].offset_y {
        return Some((0, 0));
    }
    for (i, block) in state.blocks.iter().enumerate() {
        let top = block.offset_y;
        let bot = top + block.height;
        if pos.y >= top && pos.y <= bot {
            let local = Point::new(pos.x - block.offset_x, pos.y - block.offset_y);
            let byte = match block.paragraph.hit_test(local) {
                Some(Hit::CharOffset(b)) => b,
                None => {
                    // Clamp to start or end depending on horizontal side.
                    if local.x < 0.0 {
                        0
                    } else {
                        block.total_bytes
                    }
                }
            };
            return Some((i, byte));
        }
        // In the gap between blocks: assign to whichever is closer.
        if let Some(next) = state.blocks.get(i + 1) {
            let gap_top = bot;
            let gap_bot = next.offset_y;
            if pos.y > gap_top && pos.y < gap_bot {
                let mid = (gap_top + gap_bot) / 2.0;
                return if pos.y < mid {
                    Some((i, block.total_bytes))
                } else {
                    Some((i + 1, 0))
                };
            }
        }
    }
    let last = state.blocks.len() - 1;
    Some((last, state.blocks[last].total_bytes))
}

fn hit_span_in<P: Paragraph>(
    state: &State<P>,
    pos: Point,
) -> Option<(usize, usize)> {
    for (i, block) in state.blocks.iter().enumerate() {
        let top = block.offset_y;
        let bot = top + block.height;
        if pos.y >= top && pos.y <= bot {
            let local = Point::new(pos.x - block.offset_x, pos.y - block.offset_y);
            return block.paragraph.hit_span(local).map(|s| (i, s));
        }
    }
    None
}

fn order(a: (usize, usize), b: (usize, usize)) -> ((usize, usize), (usize, usize)) {
    if block_byte_cmp(a, b) <= 0 { (a, b) } else { (b, a) }
}

fn block_byte_cmp(a: (usize, usize), b: (usize, usize)) -> isize {
    if a.0 != b.0 {
        a.0 as isize - b.0 as isize
    } else {
        a.1 as isize - b.1 as isize
    }
}

fn span_bytes<Link, Font>(spans: &[Span<'_, Link, Font>]) -> usize {
    spans.iter().map(|s| s.text.as_ref().len()).sum()
}

fn concat_spans<Link, Font>(spans: &[Span<'_, Link, Font>]) -> String {
    let mut out = String::with_capacity(span_bytes(spans));
    for s in spans {
        out.push_str(s.text.as_ref());
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

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn word_range_at(text: &str, byte_offset: usize) -> Range<usize> {
    let mut idx = byte_offset.min(text.len());
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    let focus = text[idx..]
        .chars()
        .next()
        .or_else(|| text[..idx].chars().next_back());
    let Some(ch) = focus else {
        return idx..idx;
    };
    if !is_word_char(ch) {
        return idx..idx;
    }
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

fn paint_selection_in_block<Renderer, P>(
    renderer: &mut Renderer,
    paragraph: &P,
    spans: &[Span<'static, Url>],
    range: Range<usize>,
    translation: Vector,
    color: Color,
) where
    Renderer: advanced::text::Renderer<Font = Font>,
    P: Paragraph,
{
    let mut cursor_byte = 0usize;
    for (i, span) in spans.iter().enumerate() {
        let len = span.text.as_ref().len();
        let span_range = cursor_byte..cursor_byte + len;
        cursor_byte += len;

        let overlap_start = range.start.max(span_range.start);
        let overlap_end = range.end.min(span_range.end);
        if overlap_start >= overlap_end {
            continue;
        }
        for bounds in paragraph.span_bounds(i) {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: bounds + translation,
                    ..Default::default()
                },
                color,
            );
        }
    }
}

fn extract_selection<P: Paragraph>(
    blocks: &[BlockLayout<P>],
    sel: &Selection,
) -> String {
    let (start, end) = order(sel.start, sel.end);
    let mut out = String::new();
    for (bi, block) in blocks.iter().enumerate() {
        if bi < start.0 || bi > end.0 {
            continue;
        }
        let byte_start = if bi == start.0 { start.1 } else { 0 };
        let byte_end = if bi == end.0 { end.1 } else { block.total_bytes };
        if byte_start >= byte_end {
            if bi > start.0 && bi < end.0 {
                out.push('\n');
            }
            continue;
        }
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        // If this block has a bullet (list item), include it in the copied text.
        if let Some((_, bullet, _)) = &block.bullet {
            if byte_start == 0 {
                out.push_str(bullet);
                out.push(' ');
            }
        }
        let mut cursor = 0usize;
        for span in block.spans.iter() {
            let text = span.text.as_ref();
            let len = text.len();
            let span_range = cursor..cursor + len;
            let overlap_start = byte_start.max(span_range.start);
            let overlap_end = byte_end.min(span_range.end);
            if overlap_start < overlap_end {
                let lo = overlap_start - cursor;
                let hi = overlap_end - cursor;
                let lo = floor_char_boundary(text, lo);
                let hi = floor_char_boundary(text, hi);
                out.push_str(&text[lo..hi]);
            }
            cursor += len;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Element conversion
// ---------------------------------------------------------------------------

impl<'a, Theme, Renderer> From<SelectableBubble<'a, Theme, Renderer>>
    for Element<'a, Url, Theme, Renderer>
where
    Theme: Catalog + 'a,
    Renderer: advanced::text::Renderer<Font = Font> + 'a,
{
    fn from(
        widget: SelectableBubble<'a, Theme, Renderer>,
    ) -> Element<'a, Url, Theme, Renderer> {
        Element::new(widget)
    }
}

// Silence unused-imports when `markdown` module is not further referenced —
// keeps the import line as a pointer to where these types live.
#[allow(dead_code)]
fn _force_imports(_: markdown::Item) {}
