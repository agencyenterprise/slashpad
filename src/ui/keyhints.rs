//! Bottom keyhints bar — Raycast-style footer that lists the keyboard
//! shortcuts available in the current mode.
//!
//! The bar is mounted as the last child in the palette's main Column in
//! `app.rs::view()`, and its fixed height is accounted for in
//! `app.rs::target_height()`.

use iced::widget::{container, horizontal_space, row, text, Row};
use iced::{Element, Length};

use crate::app::Message;
use crate::state::Mode;

use super::theme;

/// Extra palette-state needed to decide which hints to show. Kept narrow
/// on purpose — we don't want this view borrowing `&App`.
#[derive(Debug, Clone, Copy)]
pub struct KeyhintContext {
    /// True when the Idle list has at least one visible row (active
    /// chats or past sessions, after the live fuzzy filter).
    pub has_rows: bool,
    /// True when an idle-list row is currently highlighted — either
    /// because the input is empty (default selection) or because the
    /// user arrow-keyed into the filtered list. Drives the "Open" vs
    /// "Send" hint label on Enter.
    pub selection_active: bool,
    /// True when the active Chatting chat has a `session_id` populated
    /// (i.e., the sidecar has streamed at least one response). Gates
    /// the "⌘T Terminal" hint so it only shows when Cmd+T would
    /// actually resolve to a resumable session.
    pub has_session_id: bool,
}

/// Fixed footer height reserved for the keyhints bar, used by
/// `target_height()` in `app.rs`. Includes the bar itself (~26px:
/// 6+6 padding + 11px text + 1px border top/bottom) plus the 4px
/// `Column` spacing above it and a small safety margin so the bar
/// isn't clipped in the densest layout (Skills with filter list at
/// the 260px cap, Chatting with the 480px chat panel).
pub const BAR_HEIGHT: f32 = 40.0;

pub fn view(mode: Mode, ctx: KeyhintContext) -> Element<'static, Message> {
    let hints: Vec<(&'static str, &'static str)> = match mode {
        Mode::Idle if ctx.selection_active && ctx.has_rows => vec![
            ("↵", "Open"),
            ("↑↓", "Navigate"),
            ("/", "Skills"),
            ("esc", "Dismiss"),
        ],
        Mode::Idle if ctx.has_rows => vec![
            ("↵", "Send"),
            ("↑↓", "Navigate"),
            ("/", "Skills"),
            ("esc", "Dismiss"),
        ],
        Mode::Idle => vec![("↵", "Send"), ("/", "Skills"), ("esc", "Dismiss")],
        Mode::Skills => vec![("↵", "Run"), ("↑↓", "Navigate"), ("esc", "Dismiss")],
        Mode::Chatting if ctx.has_session_id => {
            vec![("↵", "Send"), ("⌘T", "Terminal"), ("esc", "Back")]
        }
        Mode::Chatting => vec![("↵", "Send"), ("esc", "Back")],
        Mode::Settings => vec![],
    };

    // Split: `esc` hints render flush-left, everything else flush-right.
    let (left, right): (Vec<_>, Vec<_>) = hints.into_iter().partition(|(key, _)| *key == "esc");

    let mut bar: Row<'static, Message> = Row::new().spacing(12).align_y(iced::Alignment::Center);
    for (key, label) in left {
        bar = bar.push(hint_item(key, label));
    }
    bar = bar.push(horizontal_space().width(Length::Fill));
    for (key, label) in right {
        bar = bar.push(hint_item(key, label));
    }

    container(bar)
        .padding([6, 12])
        .width(Length::Fill)
        .style(|_theme: &iced::Theme| iced::widget::container::Style {
            background: None,
            border: iced::Border {
                color: iced::Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            text_color: Some(theme::TEXT),
            ..Default::default()
        })
        .into()
}

fn hint_item(key: &'static str, label: &'static str) -> Element<'static, Message> {
    let kbd = container(text(key).size(11).color(theme::TEXT))
        .padding([2, 6])
        .style(|_theme: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::SURFACE_2)),
            border: iced::Border {
                color: theme::SURFACE_3,
                width: 1.0,
                radius: 4.0.into(),
            },
            text_color: Some(theme::TEXT),
            ..Default::default()
        });

    row![kbd, text(label).size(11).color(theme::MUTED)]
        .spacing(6)
        .align_y(iced::Alignment::Center)
        .into()
}
