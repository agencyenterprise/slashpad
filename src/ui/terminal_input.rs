//! Forked text input with a configurable block cursor.
//!
//! This is a trimmed copy of `iced_widget::text_input` (0.13.4), adapted to
//! expose `cursor_width` and `cursor_color` so the cursor can be drawn as a
//! thicker, accent-colored block — like a terminal.
//!
//! Deleted from the upstream source because the app does not use them:
//! `Icon`, `is_secure`, `on_paste`, `align_x`, `line_height`, plus the
//! free-function task helpers (`focus`, `move_cursor_to_end`, ...). The app
//! reuses `iced::widget::text_input::{focus, move_cursor_to_end, ..., Id,
//! Status, Style, Catalog, StyleFn}`; those operate on any widget that
//! registers `operation.focusable` / `operation.text_input` with the same
//! `widget::Id`, so this fork stays interoperable.
//!
//! The only meaningful logic change vs upstream is in `draw_cursor`.

use iced::advanced::clipboard::{self, Clipboard};
use iced::advanced::layout;
use iced::advanced::mouse::{self, click};
use iced::advanced::renderer;
use iced::advanced::text::paragraph::{self, Paragraph as _};
use iced::advanced::text::{self, Text};
use iced::advanced::widget::operation::{self, Operation};
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{Layout, Shell, Widget};
use iced::alignment;
use iced::event::{self, Event};
use iced::keyboard;
use iced::keyboard::key;
use iced::time::{Duration, Instant};
use iced::touch;
use iced::widget::text_input::{Catalog, Status, Style};
use iced::window;
use iced::{
    Color, Element, Length, Padding, Pixels, Point, Rectangle, Size, Vector,
};

pub use iced::widget::text_input::Id;

mod value {
    use unicode_segmentation::UnicodeSegmentation;

    #[derive(Debug, Clone, Default)]
    pub struct Value {
        graphemes: Vec<String>,
    }

    impl Value {
        pub fn new(string: &str) -> Self {
            let graphemes = UnicodeSegmentation::graphemes(string, true)
                .map(String::from)
                .collect();
            Self { graphemes }
        }

        pub fn len(&self) -> usize {
            self.graphemes.len()
        }

        pub fn previous_start_of_word(&self, index: usize) -> usize {
            let previous_string =
                &self.graphemes[..index.min(self.graphemes.len())].concat();

            UnicodeSegmentation::split_word_bound_indices(previous_string as &str)
                .rfind(|(_, word)| !word.trim_start().is_empty())
                .map(|(i, previous_word)| {
                    index
                        - UnicodeSegmentation::graphemes(previous_word, true).count()
                        - UnicodeSegmentation::graphemes(
                            &previous_string[i + previous_word.len()..] as &str,
                            true,
                        )
                        .count()
                })
                .unwrap_or(0)
        }

        pub fn next_end_of_word(&self, index: usize) -> usize {
            let next_string = &self.graphemes[index..].concat();

            UnicodeSegmentation::split_word_bound_indices(next_string as &str)
                .find(|(_, word)| !word.trim_start().is_empty())
                .map(|(i, next_word)| {
                    index
                        + UnicodeSegmentation::graphemes(next_word, true).count()
                        + UnicodeSegmentation::graphemes(
                            &next_string[..i] as &str,
                            true,
                        )
                        .count()
                })
                .unwrap_or(self.len())
        }

        pub fn select(&self, start: usize, end: usize) -> Self {
            let graphemes = self.graphemes
                [start.min(self.len())..end.min(self.len())]
                .to_vec();
            Self { graphemes }
        }

        pub fn insert(&mut self, index: usize, c: char) {
            self.graphemes.insert(index, c.to_string());
            self.graphemes =
                UnicodeSegmentation::graphemes(&self.to_string() as &str, true)
                    .map(String::from)
                    .collect();
        }

        pub fn insert_many(&mut self, index: usize, mut value: Value) {
            let _ = self
                .graphemes
                .splice(index..index, value.graphemes.drain(..));
        }

        pub fn remove(&mut self, index: usize) {
            let _ = self.graphemes.remove(index);
        }

        pub fn remove_many(&mut self, start: usize, end: usize) {
            let _ = self.graphemes.splice(start..end, std::iter::empty());
        }
    }

    impl std::fmt::Display for Value {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.graphemes.concat())
        }
    }
}

mod cursor {
    use super::value::Value;

    #[derive(Debug, Copy, Clone)]
    pub struct Cursor {
        state: State,
    }

    #[derive(Debug, Copy, Clone)]
    pub enum State {
        Index(usize),
        Selection { start: usize, end: usize },
    }

    impl Default for Cursor {
        fn default() -> Self {
            Cursor {
                state: State::Index(0),
            }
        }
    }

    impl Cursor {
        pub fn state(&self, value: &Value) -> State {
            match self.state {
                State::Index(index) => State::Index(index.min(value.len())),
                State::Selection { start, end } => {
                    let start = start.min(value.len());
                    let end = end.min(value.len());
                    if start == end {
                        State::Index(start)
                    } else {
                        State::Selection { start, end }
                    }
                }
            }
        }

        pub fn selection(&self, value: &Value) -> Option<(usize, usize)> {
            match self.state(value) {
                State::Selection { start, end } => {
                    Some((start.min(end), start.max(end)))
                }
                State::Index(_) => None,
            }
        }

        pub(super) fn move_to(&mut self, position: usize) {
            self.state = State::Index(position);
        }

        pub(super) fn move_right(&mut self, value: &Value) {
            self.move_right_by_amount(value, 1);
        }

        pub(super) fn move_right_by_words(&mut self, value: &Value) {
            self.move_to(value.next_end_of_word(self.right(value)));
        }

        pub(super) fn move_right_by_amount(
            &mut self,
            value: &Value,
            amount: usize,
        ) {
            match self.state(value) {
                State::Index(index) => {
                    self.move_to(index.saturating_add(amount).min(value.len()));
                }
                State::Selection { start, end } => self.move_to(end.max(start)),
            }
        }

        pub(super) fn move_left(&mut self, value: &Value) {
            match self.state(value) {
                State::Index(index) if index > 0 => self.move_to(index - 1),
                State::Selection { start, end } => self.move_to(start.min(end)),
                State::Index(_) => self.move_to(0),
            }
        }

        pub(super) fn move_left_by_words(&mut self, value: &Value) {
            self.move_to(value.previous_start_of_word(self.left(value)));
        }

        pub(super) fn select_range(&mut self, start: usize, end: usize) {
            if start == end {
                self.state = State::Index(start);
            } else {
                self.state = State::Selection { start, end };
            }
        }

        pub(super) fn select_left(&mut self, value: &Value) {
            match self.state(value) {
                State::Index(index) if index > 0 => {
                    self.select_range(index, index - 1);
                }
                State::Selection { start, end } if end > 0 => {
                    self.select_range(start, end - 1);
                }
                _ => {}
            }
        }

        pub(super) fn select_right(&mut self, value: &Value) {
            match self.state(value) {
                State::Index(index) if index < value.len() => {
                    self.select_range(index, index + 1);
                }
                State::Selection { start, end } if end < value.len() => {
                    self.select_range(start, end + 1);
                }
                _ => {}
            }
        }

        pub(super) fn select_left_by_words(&mut self, value: &Value) {
            match self.state(value) {
                State::Index(index) => {
                    self.select_range(index, value.previous_start_of_word(index));
                }
                State::Selection { start, end } => {
                    self.select_range(start, value.previous_start_of_word(end));
                }
            }
        }

        pub(super) fn select_right_by_words(&mut self, value: &Value) {
            match self.state(value) {
                State::Index(index) => {
                    self.select_range(index, value.next_end_of_word(index));
                }
                State::Selection { start, end } => {
                    self.select_range(start, value.next_end_of_word(end));
                }
            }
        }

        pub(super) fn select_all(&mut self, value: &Value) {
            self.select_range(0, value.len());
        }

        pub(super) fn start(&self, value: &Value) -> usize {
            let start = match self.state {
                State::Index(index) => index,
                State::Selection { start, .. } => start,
            };
            start.min(value.len())
        }

        pub(super) fn end(&self, value: &Value) -> usize {
            let end = match self.state {
                State::Index(index) => index,
                State::Selection { end, .. } => end,
            };
            end.min(value.len())
        }

        fn left(&self, value: &Value) -> usize {
            match self.state(value) {
                State::Index(index) => index,
                State::Selection { start, end } => start.min(end),
            }
        }

        fn right(&self, value: &Value) -> usize {
            match self.state(value) {
                State::Index(index) => index,
                State::Selection { start, end } => start.max(end),
            }
        }
    }
}

mod editor {
    use super::cursor::Cursor;
    use super::value::Value;

    pub struct Editor<'a> {
        value: &'a mut Value,
        cursor: &'a mut Cursor,
    }

    impl<'a> Editor<'a> {
        pub fn new(value: &'a mut Value, cursor: &'a mut Cursor) -> Editor<'a> {
            Editor { value, cursor }
        }

        pub fn contents(&self) -> String {
            self.value.to_string()
        }

        pub fn insert(&mut self, character: char) {
            if let Some((left, right)) = self.cursor.selection(self.value) {
                self.cursor.move_left(self.value);
                self.value.remove_many(left, right);
            }
            self.value.insert(self.cursor.end(self.value), character);
            self.cursor.move_right(self.value);
        }

        pub fn paste(&mut self, content: Value) {
            let length = content.len();
            if let Some((left, right)) = self.cursor.selection(self.value) {
                self.cursor.move_left(self.value);
                self.value.remove_many(left, right);
            }
            self.value.insert_many(self.cursor.end(self.value), content);
            self.cursor.move_right_by_amount(self.value, length);
        }

        pub fn backspace(&mut self) {
            match self.cursor.selection(self.value) {
                Some((start, end)) => {
                    self.cursor.move_left(self.value);
                    self.value.remove_many(start, end);
                }
                None => {
                    let start = self.cursor.start(self.value);
                    if start > 0 {
                        self.cursor.move_left(self.value);
                        self.value.remove(start - 1);
                    }
                }
            }
        }

        pub fn delete(&mut self) {
            match self.cursor.selection(self.value) {
                Some(_) => {
                    self.backspace();
                }
                None => {
                    let end = self.cursor.end(self.value);
                    if end < self.value.len() {
                        self.value.remove(end);
                    }
                }
            }
        }
    }
}

use cursor::Cursor;
use editor::Editor;
use value::Value;

const DEFAULT_PADDING: Padding = Padding::new(5.0);
const CURSOR_BLINK_INTERVAL_MILLIS: u128 = 500;

/// A single-line text input with a configurable block cursor.
#[allow(missing_debug_implementations)]
pub struct TerminalInput<
    'a,
    Message,
    Theme = iced::Theme,
    Renderer = iced::Renderer,
> where
    Theme: Catalog,
    Renderer: text::Renderer,
{
    id: Option<iced::advanced::widget::Id>,
    placeholder: String,
    value: Value,
    font: Option<Renderer::Font>,
    width: Length,
    padding: Padding,
    size: Option<Pixels>,
    on_input: Option<Box<dyn Fn(String) -> Message + 'a>>,
    on_submit: Option<Message>,
    cursor_width: f32,
    cursor_color: Option<Color>,
    class: Theme::Class<'a>,
}

impl<'a, Message, Theme, Renderer> TerminalInput<'a, Message, Theme, Renderer>
where
    Message: Clone,
    Theme: Catalog,
    Renderer: text::Renderer,
{
    pub fn new(placeholder: &str, value: &str) -> Self {
        Self {
            id: None,
            placeholder: String::from(placeholder),
            value: Value::new(value),
            font: None,
            width: Length::Fill,
            padding: DEFAULT_PADDING,
            size: None,
            on_input: None,
            on_submit: None,
            cursor_width: 1.0,
            cursor_color: None,
            class: Theme::default(),
        }
    }

    pub fn id(mut self, id: impl Into<Id>) -> Self {
        self.id = Some(id.into().into());
        self
    }

    pub fn on_input(
        mut self,
        on_input: impl Fn(String) -> Message + 'a,
    ) -> Self {
        self.on_input = Some(Box::new(on_input));
        self
    }

    pub fn on_submit(mut self, message: Message) -> Self {
        self.on_submit = Some(message);
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn padding<P: Into<Padding>>(mut self, padding: P) -> Self {
        self.padding = padding.into();
        self
    }

    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = Some(size.into());
        self
    }

    /// Sets the width of the blinking cursor in logical pixels.
    pub fn cursor_width(mut self, width: f32) -> Self {
        self.cursor_width = width;
        self
    }

    /// Overrides the cursor color. Defaults to `style.value` (text color).
    pub fn cursor_color(mut self, color: Color) -> Self {
        self.cursor_color = Some(color);
        self
    }

    #[must_use]
    pub fn style(
        mut self,
        style: impl Fn(&Theme, Status) -> Style + 'a,
    ) -> Self
    where
        Theme::Class<'a>: From<iced::widget::text_input::StyleFn<'a, Theme>>,
    {
        self.class = (Box::new(style)
            as iced::widget::text_input::StyleFn<'a, Theme>)
            .into();
        self
    }

    fn layout(
        &self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
        value: Option<&Value>,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State<Renderer::Paragraph>>();
        let value = value.unwrap_or(&self.value);

        let font = self.font.unwrap_or_else(|| renderer.default_font());
        let text_size = self.size.unwrap_or_else(|| renderer.default_size());
        let padding = self.padding.fit(Size::ZERO, limits.max());
        let line_height = text::LineHeight::default();
        let height = line_height.to_absolute(text_size);

        let limits = limits.width(self.width).shrink(padding);
        let text_bounds = limits.resolve(self.width, height, Size::ZERO);

        let placeholder_text = Text {
            font,
            line_height,
            content: self.placeholder.as_str(),
            bounds: Size::new(f32::INFINITY, text_bounds.height),
            size: text_size,
            horizontal_alignment: alignment::Horizontal::Left,
            vertical_alignment: alignment::Vertical::Center,
            shaping: text::Shaping::Advanced,
            wrapping: text::Wrapping::default(),
        };

        state.placeholder.update(placeholder_text);

        state.value.update(Text {
            content: &value.to_string(),
            ..placeholder_text
        });

        let text = layout::Node::new(text_bounds)
            .move_to(Point::new(padding.left, padding.top));

        layout::Node::with_children(text_bounds.expand(padding), vec![text])
    }

    fn draw_inner(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State<Renderer::Paragraph>>();
        let value = &self.value;
        let is_disabled = self.on_input.is_none();

        let bounds = layout.bounds();
        let mut children_layout = layout.children();
        let text_bounds = children_layout.next().unwrap().bounds();
        let is_mouse_over = cursor.is_over(bounds);

        let status = if is_disabled {
            Status::Disabled
        } else if state.is_focused() {
            Status::Focused
        } else if is_mouse_over {
            Status::Hovered
        } else {
            Status::Active
        };

        let style = theme.style(&self.class, status);

        renderer.fill_quad(
            renderer::Quad {
                bounds,
                border: style.border,
                ..renderer::Quad::default()
            },
            style.background,
        );

        let text = value.to_string();

        let cursor_color = self.cursor_color.unwrap_or(style.value);
        let cursor_width = self.cursor_width.max(0.0);

        let (cursor_draw, offset, is_selecting) = if let Some(focus) = state
            .is_focused
            .as_ref()
            .filter(|focus| focus.is_window_focused)
        {
            match state.cursor.state(value) {
                cursor::State::Index(position) => {
                    let (text_value_width, offset) =
                        measure_cursor_and_scroll_offset(
                            state.value.raw(),
                            text_bounds,
                            position,
                        );

                    let is_cursor_visible = !is_disabled
                        && ((focus.now - focus.updated_at).as_millis()
                            / CURSOR_BLINK_INTERVAL_MILLIS)
                            .is_multiple_of(2);

                    let cursor_draw = if is_cursor_visible {
                        Some((
                            renderer::Quad {
                                bounds: Rectangle {
                                    x: (text_bounds.x + text_value_width)
                                        .floor(),
                                    y: text_bounds.y,
                                    width: cursor_width,
                                    height: text_bounds.height,
                                },
                                ..renderer::Quad::default()
                            },
                            cursor_color,
                        ))
                    } else {
                        None
                    };

                    (cursor_draw, offset, false)
                }
                cursor::State::Selection { start, end } => {
                    let left = start.min(end);
                    let right = end.max(start);

                    let (left_position, left_offset) =
                        measure_cursor_and_scroll_offset(
                            state.value.raw(),
                            text_bounds,
                            left,
                        );
                    let (right_position, right_offset) =
                        measure_cursor_and_scroll_offset(
                            state.value.raw(),
                            text_bounds,
                            right,
                        );

                    let width = right_position - left_position;

                    (
                        Some((
                            renderer::Quad {
                                bounds: Rectangle {
                                    x: text_bounds.x + left_position,
                                    y: text_bounds.y,
                                    width,
                                    height: text_bounds.height,
                                },
                                ..renderer::Quad::default()
                            },
                            style.selection,
                        )),
                        if end == right {
                            right_offset
                        } else {
                            left_offset
                        },
                        true,
                    )
                }
            }
        } else {
            (None, 0.0, false)
        };

        let alignment = alignment::Horizontal::Left;

        let draw = |renderer: &mut Renderer, viewport| {
            let paragraph = if text.is_empty() {
                state.placeholder.raw()
            } else {
                state.value.raw()
            };

            let alignment_offset = alignment_offset(
                text_bounds.width,
                paragraph.min_width(),
                alignment,
            );

            if let Some((cursor_quad, color)) = cursor_draw {
                renderer.with_translation(
                    Vector::new(alignment_offset - offset, 0.0),
                    |renderer| {
                        renderer.fill_quad(cursor_quad, color);
                    },
                );
            } else {
                renderer.with_translation(Vector::ZERO, |_| {});
            }

            renderer.fill_paragraph(
                paragraph,
                Point::new(text_bounds.x, text_bounds.center_y())
                    + Vector::new(alignment_offset - offset, 0.0),
                if text.is_empty() {
                    style.placeholder
                } else {
                    style.value
                },
                viewport,
            );
        };

        if is_selecting {
            renderer
                .with_layer(text_bounds, |renderer| draw(renderer, *viewport));
        } else {
            draw(renderer, text_bounds);
        }
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for TerminalInput<'a, Message, Theme, Renderer>
where
    Message: Clone,
    Theme: Catalog,
    Renderer: text::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<Renderer::Paragraph>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::<Renderer::Paragraph>::new())
    }

    fn diff(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State<Renderer::Paragraph>>();
        if self.on_input.is_none() {
            state.is_pasting = None;
        }
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
        Self::layout(self, tree, renderer, limits, None)
    }

    fn operate(
        &self,
        tree: &mut Tree,
        _layout: Layout<'_>,
        _renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let state = tree.state.downcast_mut::<State<Renderer::Paragraph>>();
        operation.focusable(state, self.id.as_ref());
        operation.text_input(state, self.id.as_ref());
    }

    fn on_event(
        &mut self,
        tree: &mut Tree,
        event: Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) -> event::Status {
        let line_height = text::LineHeight::default();
        let update_cache = |state, value| {
            replace_paragraph(
                renderer,
                state,
                layout,
                value,
                self.font,
                self.size,
                line_height,
            );
        };

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerPressed { .. }) => {
                let state = state_mut::<Renderer>(tree);
                let click_position = cursor.position_over(layout.bounds());

                state.is_focused = if click_position.is_some() {
                    state.is_focused.or_else(|| {
                        let now = Instant::now();
                        Some(Focus {
                            updated_at: now,
                            now,
                            is_window_focused: true,
                        })
                    })
                } else {
                    None
                };

                if let Some(cursor_position) = click_position {
                    let text_layout = layout.children().next().unwrap();

                    let target = {
                        let text_bounds = text_layout.bounds();
                        let alignment_offset = alignment_offset(
                            text_bounds.width,
                            state.value.raw().min_width(),
                            alignment::Horizontal::Left,
                        );
                        cursor_position.x - text_bounds.x - alignment_offset
                    };

                    let click = mouse::Click::new(
                        cursor_position,
                        mouse::Button::Left,
                        state.last_click,
                    );

                    match click.kind() {
                        click::Kind::Single => {
                            let position = if target > 0.0 {
                                find_cursor_position(
                                    text_layout.bounds(),
                                    &self.value,
                                    state,
                                    target,
                                )
                            } else {
                                None
                            }
                            .unwrap_or(0);

                            if state.keyboard_modifiers.shift() {
                                state.cursor.select_range(
                                    state.cursor.start(&self.value),
                                    position,
                                );
                            } else {
                                state.cursor.move_to(position);
                            }
                            state.is_dragging = true;
                        }
                        click::Kind::Double => {
                            let position = find_cursor_position(
                                text_layout.bounds(),
                                &self.value,
                                state,
                                target,
                            )
                            .unwrap_or(0);

                            state.cursor.select_range(
                                self.value.previous_start_of_word(position),
                                self.value.next_end_of_word(position),
                            );
                            state.is_dragging = false;
                        }
                        click::Kind::Triple => {
                            state.cursor.select_all(&self.value);
                            state.is_dragging = false;
                        }
                    }

                    state.last_click = Some(click);
                    return event::Status::Captured;
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerLifted { .. })
            | Event::Touch(touch::Event::FingerLost { .. }) => {
                state_mut::<Renderer>(tree).is_dragging = false;
            }
            Event::Mouse(mouse::Event::CursorMoved { position })
            | Event::Touch(touch::Event::FingerMoved { position, .. }) => {
                let state = state_mut::<Renderer>(tree);
                if state.is_dragging {
                    let text_layout = layout.children().next().unwrap();
                    let target = {
                        let text_bounds = text_layout.bounds();
                        let alignment_offset = alignment_offset(
                            text_bounds.width,
                            state.value.raw().min_width(),
                            alignment::Horizontal::Left,
                        );
                        position.x - text_bounds.x - alignment_offset
                    };

                    let position = find_cursor_position(
                        text_layout.bounds(),
                        &self.value,
                        state,
                        target,
                    )
                    .unwrap_or(0);

                    state
                        .cursor
                        .select_range(state.cursor.start(&self.value), position);

                    return event::Status::Captured;
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key, text, ..
            }) => {
                let state = state_mut::<Renderer>(tree);

                if let Some(focus) = &mut state.is_focused {
                    let modifiers = state.keyboard_modifiers;
                    focus.updated_at = Instant::now();

                    match key.as_ref() {
                        keyboard::Key::Character("c")
                            if state.keyboard_modifiers.command() =>
                        {
                            if let Some((start, end)) =
                                state.cursor.selection(&self.value)
                            {
                                clipboard.write(
                                    clipboard::Kind::Standard,
                                    self.value.select(start, end).to_string(),
                                );
                            }
                            return event::Status::Captured;
                        }
                        keyboard::Key::Character("x")
                            if state.keyboard_modifiers.command() =>
                        {
                            let Some(on_input) = &self.on_input else {
                                return event::Status::Ignored;
                            };

                            if let Some((start, end)) =
                                state.cursor.selection(&self.value)
                            {
                                clipboard.write(
                                    clipboard::Kind::Standard,
                                    self.value.select(start, end).to_string(),
                                );
                            }

                            let mut editor =
                                Editor::new(&mut self.value, &mut state.cursor);
                            editor.delete();

                            let message = (on_input)(editor.contents());
                            shell.publish(message);

                            update_cache(state, &self.value);
                            return event::Status::Captured;
                        }
                        keyboard::Key::Character("v")
                            if state.keyboard_modifiers.command()
                                && !state.keyboard_modifiers.alt() =>
                        {
                            let Some(on_input) = &self.on_input else {
                                return event::Status::Ignored;
                            };

                            let content = match state.is_pasting.take() {
                                Some(content) => content,
                                None => {
                                    let content: String = clipboard
                                        .read(clipboard::Kind::Standard)
                                        .unwrap_or_default()
                                        .chars()
                                        .filter(|c| !c.is_control())
                                        .collect();
                                    Value::new(&content)
                                }
                            };

                            let mut editor =
                                Editor::new(&mut self.value, &mut state.cursor);
                            editor.paste(content.clone());

                            let message = (on_input)(editor.contents());
                            shell.publish(message);

                            state.is_pasting = Some(content);

                            update_cache(state, &self.value);
                            return event::Status::Captured;
                        }
                        keyboard::Key::Character("a")
                            if state.keyboard_modifiers.command() =>
                        {
                            state.cursor.select_all(&self.value);
                            return event::Status::Captured;
                        }
                        _ => {}
                    }

                    if let Some(text) = text {
                        let Some(on_input) = &self.on_input else {
                            return event::Status::Ignored;
                        };

                        state.is_pasting = None;

                        if let Some(c) =
                            text.chars().next().filter(|c| !c.is_control())
                        {
                            let mut editor =
                                Editor::new(&mut self.value, &mut state.cursor);
                            editor.insert(c);

                            let message = (on_input)(editor.contents());
                            shell.publish(message);

                            focus.updated_at = Instant::now();
                            update_cache(state, &self.value);
                            return event::Status::Captured;
                        }
                    }

                    match key.as_ref() {
                        keyboard::Key::Named(key::Named::Enter) => {
                            if let Some(on_submit) = self.on_submit.clone() {
                                shell.publish(on_submit);
                            }
                        }
                        keyboard::Key::Named(key::Named::Backspace) => {
                            let Some(on_input) = &self.on_input else {
                                return event::Status::Ignored;
                            };

                            if modifiers.jump()
                                && state.cursor.selection(&self.value).is_none()
                            {
                                state.cursor.select_left_by_words(&self.value);
                            }

                            let mut editor =
                                Editor::new(&mut self.value, &mut state.cursor);
                            editor.backspace();

                            let message = (on_input)(editor.contents());
                            shell.publish(message);
                            update_cache(state, &self.value);
                        }
                        keyboard::Key::Named(key::Named::Delete) => {
                            let Some(on_input) = &self.on_input else {
                                return event::Status::Ignored;
                            };

                            if modifiers.jump()
                                && state.cursor.selection(&self.value).is_none()
                            {
                                state.cursor.select_right_by_words(&self.value);
                            }

                            let mut editor =
                                Editor::new(&mut self.value, &mut state.cursor);
                            editor.delete();

                            let message = (on_input)(editor.contents());
                            shell.publish(message);
                            update_cache(state, &self.value);
                        }
                        keyboard::Key::Named(key::Named::Home) => {
                            if modifiers.shift() {
                                state.cursor.select_range(
                                    state.cursor.start(&self.value),
                                    0,
                                );
                            } else {
                                state.cursor.move_to(0);
                            }
                        }
                        keyboard::Key::Named(key::Named::End) => {
                            if modifiers.shift() {
                                state.cursor.select_range(
                                    state.cursor.start(&self.value),
                                    self.value.len(),
                                );
                            } else {
                                state.cursor.move_to(self.value.len());
                            }
                        }
                        keyboard::Key::Named(key::Named::ArrowLeft)
                            if modifiers.macos_command() =>
                        {
                            if modifiers.shift() {
                                state.cursor.select_range(
                                    state.cursor.start(&self.value),
                                    0,
                                );
                            } else {
                                state.cursor.move_to(0);
                            }
                        }
                        keyboard::Key::Named(key::Named::ArrowRight)
                            if modifiers.macos_command() =>
                        {
                            if modifiers.shift() {
                                state.cursor.select_range(
                                    state.cursor.start(&self.value),
                                    self.value.len(),
                                );
                            } else {
                                state.cursor.move_to(self.value.len());
                            }
                        }
                        keyboard::Key::Named(key::Named::ArrowLeft) => {
                            if modifiers.jump() {
                                if modifiers.shift() {
                                    state
                                        .cursor
                                        .select_left_by_words(&self.value);
                                } else {
                                    state
                                        .cursor
                                        .move_left_by_words(&self.value);
                                }
                            } else if modifiers.shift() {
                                state.cursor.select_left(&self.value);
                            } else {
                                state.cursor.move_left(&self.value);
                            }
                        }
                        keyboard::Key::Named(key::Named::ArrowRight) => {
                            if modifiers.jump() {
                                if modifiers.shift() {
                                    state
                                        .cursor
                                        .select_right_by_words(&self.value);
                                } else {
                                    state
                                        .cursor
                                        .move_right_by_words(&self.value);
                                }
                            } else if modifiers.shift() {
                                state.cursor.select_right(&self.value);
                            } else {
                                state.cursor.move_right(&self.value);
                            }
                        }
                        keyboard::Key::Named(key::Named::Escape) => {
                            state.is_focused = None;
                            state.is_dragging = false;
                            state.is_pasting = None;
                            state.keyboard_modifiers =
                                keyboard::Modifiers::default();
                        }
                        keyboard::Key::Named(
                            key::Named::Tab
                            | key::Named::ArrowUp
                            | key::Named::ArrowDown,
                        ) => {
                            return event::Status::Ignored;
                        }
                        _ => {}
                    }

                    return event::Status::Captured;
                }
            }
            Event::Keyboard(keyboard::Event::KeyReleased { key, .. }) => {
                let state = state_mut::<Renderer>(tree);
                if state.is_focused.is_some() {
                    match key.as_ref() {
                        keyboard::Key::Character("v") => {
                            state.is_pasting = None;
                        }
                        keyboard::Key::Named(
                            key::Named::Tab
                            | key::Named::ArrowUp
                            | key::Named::ArrowDown,
                        ) => {
                            return event::Status::Ignored;
                        }
                        _ => {}
                    }
                    return event::Status::Captured;
                }
                state.is_pasting = None;
            }
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state_mut::<Renderer>(tree).keyboard_modifiers = modifiers;
            }
            Event::Window(window::Event::Unfocused) => {
                let state = state_mut::<Renderer>(tree);
                if let Some(focus) = &mut state.is_focused {
                    focus.is_window_focused = false;
                }
            }
            Event::Window(window::Event::Focused) => {
                let state = state_mut::<Renderer>(tree);
                if let Some(focus) = &mut state.is_focused {
                    focus.is_window_focused = true;
                    focus.updated_at = Instant::now();
                    shell.request_redraw(window::RedrawRequest::NextFrame);
                }
            }
            Event::Window(window::Event::RedrawRequested(now)) => {
                let state = state_mut::<Renderer>(tree);
                if let Some(focus) = &mut state.is_focused {
                    if focus.is_window_focused {
                        focus.now = now;
                        let millis_until_redraw = CURSOR_BLINK_INTERVAL_MILLIS
                            - (now - focus.updated_at).as_millis()
                                % CURSOR_BLINK_INTERVAL_MILLIS;
                        shell.request_redraw(window::RedrawRequest::At(
                            now + Duration::from_millis(
                                millis_until_redraw as u64,
                            ),
                        ));
                    }
                }
            }
            _ => {}
        }

        event::Status::Ignored
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.draw_inner(tree, renderer, theme, layout, cursor, viewport);
    }

    fn mouse_interaction(
        &self,
        _state: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) {
            if self.on_input.is_none() {
                mouse::Interaction::Idle
            } else {
                mouse::Interaction::Text
            }
        } else {
            mouse::Interaction::default()
        }
    }
}

impl<'a, Message, Theme, Renderer>
    From<TerminalInput<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: Catalog + 'a,
    Renderer: text::Renderer + 'a,
{
    fn from(
        input: TerminalInput<'a, Message, Theme, Renderer>,
    ) -> Element<'a, Message, Theme, Renderer> {
        Element::new(input)
    }
}

#[derive(Debug, Default, Clone)]
struct State<P: text::Paragraph> {
    value: paragraph::Plain<P>,
    placeholder: paragraph::Plain<P>,
    is_focused: Option<Focus>,
    is_dragging: bool,
    is_pasting: Option<Value>,
    last_click: Option<mouse::Click>,
    cursor: Cursor,
    keyboard_modifiers: keyboard::Modifiers,
}

fn state_mut<Renderer: text::Renderer>(
    tree: &mut Tree,
) -> &mut State<Renderer::Paragraph> {
    tree.state.downcast_mut::<State<Renderer::Paragraph>>()
}

#[derive(Debug, Clone, Copy)]
struct Focus {
    updated_at: Instant,
    now: Instant,
    is_window_focused: bool,
}

impl<P: text::Paragraph> State<P> {
    fn new() -> Self {
        Self::default()
    }

    fn is_focused(&self) -> bool {
        self.is_focused.is_some()
    }

    fn focus(&mut self) {
        let now = Instant::now();
        self.is_focused = Some(Focus {
            updated_at: now,
            now,
            is_window_focused: true,
        });
        self.cursor.move_to(usize::MAX);
    }

    fn unfocus(&mut self) {
        self.is_focused = None;
    }

    fn move_cursor_to_front(&mut self) {
        self.cursor.move_to(0);
    }

    fn move_cursor_to_end(&mut self) {
        self.cursor.move_to(usize::MAX);
    }

    fn move_cursor_to(&mut self, position: usize) {
        self.cursor.move_to(position);
    }

    fn select_all(&mut self) {
        self.cursor.select_range(0, usize::MAX);
    }
}

impl<P: text::Paragraph> operation::Focusable for State<P> {
    fn is_focused(&self) -> bool {
        State::is_focused(self)
    }

    fn focus(&mut self) {
        State::focus(self);
    }

    fn unfocus(&mut self) {
        State::unfocus(self);
    }
}

impl<P: text::Paragraph> operation::TextInput for State<P> {
    fn move_cursor_to_front(&mut self) {
        State::move_cursor_to_front(self);
    }

    fn move_cursor_to_end(&mut self) {
        State::move_cursor_to_end(self);
    }

    fn move_cursor_to(&mut self, position: usize) {
        State::move_cursor_to(self, position);
    }

    fn select_all(&mut self) {
        State::select_all(self);
    }
}

fn offset<P: text::Paragraph>(
    text_bounds: Rectangle,
    value: &Value,
    state: &State<P>,
) -> f32 {
    if state.is_focused() {
        let cursor = state.cursor;
        let focus_position = match cursor.state(value) {
            cursor::State::Index(i) => i,
            cursor::State::Selection { end, .. } => end,
        };
        let (_, offset) = measure_cursor_and_scroll_offset(
            state.value.raw(),
            text_bounds,
            focus_position,
        );
        offset
    } else {
        0.0
    }
}

fn measure_cursor_and_scroll_offset(
    paragraph: &impl text::Paragraph,
    text_bounds: Rectangle,
    cursor_index: usize,
) -> (f32, f32) {
    let grapheme_position = paragraph
        .grapheme_position(0, cursor_index)
        .unwrap_or(Point::ORIGIN);

    let offset = ((grapheme_position.x + 5.0) - text_bounds.width).max(0.0);
    (grapheme_position.x, offset)
}

fn find_cursor_position<P: text::Paragraph>(
    text_bounds: Rectangle,
    value: &Value,
    state: &State<P>,
    x: f32,
) -> Option<usize> {
    let offset = offset(text_bounds, value, state);
    let value = value.to_string();

    let char_offset = state
        .value
        .raw()
        .hit_test(Point::new(x + offset, text_bounds.height / 2.0))
        .map(text::Hit::cursor)?;

    Some(
        unicode_segmentation::UnicodeSegmentation::graphemes(
            &value[..char_offset.min(value.len())],
            true,
        )
        .count(),
    )
}

fn replace_paragraph<Renderer>(
    renderer: &Renderer,
    state: &mut State<Renderer::Paragraph>,
    layout: Layout<'_>,
    value: &Value,
    font: Option<Renderer::Font>,
    text_size: Option<Pixels>,
    line_height: text::LineHeight,
) where
    Renderer: text::Renderer,
{
    let font = font.unwrap_or_else(|| renderer.default_font());
    let text_size = text_size.unwrap_or_else(|| renderer.default_size());

    let mut children_layout = layout.children();
    let text_bounds = children_layout.next().unwrap().bounds();

    state.value = paragraph::Plain::new(Text {
        font,
        line_height,
        content: &value.to_string(),
        bounds: Size::new(f32::INFINITY, text_bounds.height),
        size: text_size,
        horizontal_alignment: alignment::Horizontal::Left,
        vertical_alignment: alignment::Vertical::Top,
        shaping: text::Shaping::Advanced,
        wrapping: text::Wrapping::default(),
    });
}

fn alignment_offset(
    text_bounds_width: f32,
    text_min_width: f32,
    alignment: alignment::Horizontal,
) -> f32 {
    if text_min_width > text_bounds_width {
        0.0
    } else {
        match alignment {
            alignment::Horizontal::Left => 0.0,
            alignment::Horizontal::Center => {
                (text_bounds_width - text_min_width) / 2.0
            }
            alignment::Horizontal::Right => text_bounds_width - text_min_width,
        }
    }
}

