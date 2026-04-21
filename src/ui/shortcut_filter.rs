//! A widget wrapper that drops specific `KeyPressed` events before they
//! reach the inner widget.
//!
//! Why this exists: iced's `text_input` widget inserts any non-control
//! character from `KeyPressed.text` into its internal buffer and fires
//! `on_input`, *without* checking whether `Cmd` or `Ctrl` is held
//! (except for the hard-coded c/x/v/a clipboard shortcuts). That means
//! a shortcut like `Cmd+Shift+A` produces `text = Some("A")`, text_input
//! inserts the `A`, publishes `InputChanged("...A")`, and renders a
//! frame showing the leaked letter before any subscription handler can
//! strip it.
//!
//! The caller decides which events to drop via a predicate, so the
//! filter doesn't impose a policy of its own — in particular, it must
//! *not* blanket-block `Cmd`+letter, because text_input relies on
//! `Cmd+C/V/X/A` internally for clipboard and select-all.
//!
//! `listen_with` subscriptions observe events independently of
//! widget-tree propagation (proof: `Escape` dismisses the palette even
//! though `text_input` returns `Status::Captured` for Escape when
//! focused), so wrapping the input with this filter does not break
//! shortcut detection — it only prevents the letter from ever reaching
//! text_input.

use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, Widget};
use iced::advanced::{Clipboard, Layout, Shell};
use iced::event::{self, Event};
use iced::keyboard;
use iced::{Element, Length, Rectangle, Size, Vector};

/// Predicate called on every `KeyPressed` reaching the filter: returning
/// `true` drops the event before the inner widget sees it.
pub type DropPredicate<'a> = Box<dyn Fn(&keyboard::Key, keyboard::Modifiers) -> bool + 'a>;

pub struct ShortcutFilter<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    should_drop: DropPredicate<'a>,
}

impl<'a, Message, Theme, Renderer> ShortcutFilter<'a, Message, Theme, Renderer> {
    /// Wraps `content`. `should_drop(key, modifiers)` is called on every
    /// `KeyPressed` event; returning `true` drops the event before the
    /// inner widget sees it.
    pub fn new(
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        should_drop: impl Fn(&keyboard::Key, keyboard::Modifiers) -> bool + 'a,
    ) -> Self {
        Self {
            content: content.into(),
            should_drop: Box::new(should_drop),
        }
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for ShortcutFilter<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn layout(
        &self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget()
            .operate(&mut tree.children[0], layout, renderer, operation);
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
        viewport: &Rectangle,
    ) -> event::Status {
        // Drop matching KeyPressed events before the inner widget sees
        // them. The subscription side of the runtime still receives the
        // event (subscriptions are not gated by widget capture), which
        // is how shortcut routing continues to work.
        if let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = &event {
            if (self.should_drop)(key, *modifiers) {
                return event::Status::Captured;
            }
        }

        self.content.as_widget_mut().on_event(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        )
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        renderer_style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            renderer_style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            translation,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<ShortcutFilter<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: 'a + renderer::Renderer,
{
    fn from(
        filter: ShortcutFilter<'a, Message, Theme, Renderer>,
    ) -> Element<'a, Message, Theme, Renderer> {
        Element::new(filter)
    }
}
