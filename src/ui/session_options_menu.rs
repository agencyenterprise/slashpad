//! Floating options menu for a selected idle-list / skill / project row.
//! Mounted as an overlay via `iced::widget::stack` above the palette body
//! when `session_menu_open` is true.
//!
//! The menu positions itself bottom-left of the palette, hovering just
//! above the keyhints bar near the `⌘K Actions` hint. Keyboard-driven:
//! `↵` runs the selected action, `esc` / `⌘K` dismiss.

use iced::widget::{button, column, container, row, text, Column};
use iced::{Element, Length};

use crate::app::Message;

use super::keyhints::BAR_HEIGHT;
use super::theme;

/// Width of the floating menu panel. Narrow enough to feel like a
/// context popover rather than a full-width list.
const PANEL_W: f32 = 260.0;

/// Gap between the top of the keyhints bar and the bottom of the menu.
const ABOVE_BAR_GAP: f32 = 8.0;

/// Right inset for the menu so it sits above the `⌘K Actions` hint,
/// which lives in the flush-right cluster of the keyhints bar.
const RIGHT_INSET: f32 = 12.0;

/// What kind of list row the menu is acting on. Drives row labels and
/// which actions are rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuTarget {
    Session,
    Skill,
    Project,
}

impl MenuTarget {
    /// Number of rows in the menu for this target. Used by the caller
    /// to clamp `session_menu_selected` when arrow-navigating.
    pub fn row_count(self) -> usize {
        match self {
            MenuTarget::Session => 2, // Pin + Archive
            MenuTarget::Skill | MenuTarget::Project => 1, // Pin only
        }
    }

    fn pin_label(self, is_pinned: bool) -> &'static str {
        match (self, is_pinned) {
            (MenuTarget::Session, false) => "Pin session",
            (MenuTarget::Session, true) => "Unpin session",
            (MenuTarget::Skill, false) => "Pin skill",
            (MenuTarget::Skill, true) => "Unpin skill",
            (MenuTarget::Project, false) => "Pin project",
            (MenuTarget::Project, true) => "Unpin project",
        }
    }
}

/// Render the options menu as a full-size overlay layer. Intended to be
/// placed as the second child of an `iced::widget::stack` where the
/// first child is the ordinary palette body.
///
/// `can_act` is false when the target row can't actually be acted on
/// (e.g. a session row without a `session_id` yet). `menu_selected` is
/// the index of the highlighted menu row. `is_pinned` swaps the top
/// row's label between Pin and Unpin variants.
pub fn view(
    target: MenuTarget,
    menu_selected: usize,
    can_act: bool,
    is_pinned: bool,
) -> Element<'static, Message> {
    let header: Element<'static, Message> = text("Actions")
        .size(10)
        .color(theme::MUTED)
        .into();

    let pin = menu_row(
        target.pin_label(is_pinned),
        menu_selected == 0,
        can_act,
        Message::TogglePinSelectedRow,
    );

    let mut body: Column<'static, Message> = column![header, pin].spacing(6);
    if matches!(target, MenuTarget::Session) {
        let archive = menu_row(
            "Archive session",
            menu_selected == 1,
            can_act,
            Message::ArchiveSelectedRow,
        );
        body = body.push(archive);
    }

    let panel: Element<'static, Message> = container(body)
        .padding([8, 10])
        .width(Length::Fixed(PANEL_W))
        .style(|_theme: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::SURFACE_2)),
            border: iced::Border {
                color: theme::SURFACE_3,
                width: 1.0,
                radius: 8.0.into(),
            },
            text_color: Some(theme::TEXT),
            ..Default::default()
        })
        .into();

    // Outer full-size container anchors the panel bottom-right, offset
    // up by the keyhints bar + a small gap so the popover floats above
    // the `⌘K Actions` hint (which lives in the flush-right cluster).
    container(panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced::alignment::Horizontal::Right)
        .align_y(iced::alignment::Vertical::Bottom)
        .padding(iced::Padding {
            top: 0.0,
            right: RIGHT_INSET,
            bottom: BAR_HEIGHT + ABOVE_BAR_GAP,
            left: 0.0,
        })
        .into()
}

fn menu_row(
    label: &'static str,
    selected: bool,
    enabled: bool,
    on_press: Message,
) -> Element<'static, Message> {
    let color = if enabled { theme::TEXT } else { theme::MUTED };
    let body = row![text(label).size(13).color(color)]
        .spacing(8)
        .align_y(iced::Alignment::Center);

    let inner = container(body)
        .padding([6, 10])
        .width(Length::Fill)
        .style(move |_theme: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(if selected && enabled {
                theme::SURFACE_3
            } else {
                iced::Color::TRANSPARENT
            })),
            border: iced::Border {
                color: iced::Color::TRANSPARENT,
                width: 0.0,
                radius: 4.0.into(),
            },
            text_color: Some(color),
            ..Default::default()
        });

    let mut btn = button(inner)
        .padding(0)
        .width(Length::Fill)
        .style(|_theme, _status| iced::widget::button::Style {
            background: None,
            text_color: theme::TEXT,
            ..Default::default()
        });
    if enabled {
        btn = btn.on_press(on_press);
    }
    btn.into()
}
