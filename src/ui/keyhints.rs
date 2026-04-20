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
#[derive(Debug, Clone)]
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
    /// True when the active chat is currently Initializing or
    /// Streaming — i.e., a turn is in flight and Ctrl+C would cancel
    /// it. Gates the `ctrl c  Cancel` hint so it only appears when it
    /// would actually do something.
    pub is_generating: bool,
    /// True in `Mode::Skills` when the input has already committed to
    /// a concrete skill (`/<name>` or `/<name> ...`). Swaps the Enter
    /// hint from "Select" (autocomplete) to "Run".
    pub skill_locked: bool,
    /// Tilde-abbreviated display of the directory Claude Code is running
    /// in (e.g. `~/.slashpad`). Rendered centered in the bar.
    pub project_path_display: String,
    /// True when the user has pinned the palette via Cmd+Shift+P.
    /// Pinning captures the window's current position *and*, if the
    /// user pinned while viewing a chat, the chat id — summoning the
    /// palette later restores both. Swaps the hint label from "Pin"
    /// to "Unpin".
    pub pinned: bool,
    /// True when the floating `⌘K Actions` submenu is open. Replaces
    /// the normal idle-mode hints with menu-specific ones.
    pub session_menu_open: bool,
    /// True when the currently-selected idle row has a tagable
    /// `session_id`. Gates the `⌘K Actions` hint so it only appears
    /// for rows the action can actually target.
    pub can_open_options: bool,
    /// Which row within the options menu is highlighted. 0 = Pin/Unpin,
    /// 1 = Archive. Used to pick the ↵ hint label.
    pub session_menu_selected: usize,
    /// True when the currently-selected row's session is tagged pinned.
    /// Flips the Pin/Unpin hint label while the menu is open.
    pub selected_is_pinned: bool,
}

/// Fixed footer height reserved for the keyhints bar, used by
/// `target_height()` in `app.rs`. Includes the bar itself (~26px:
/// 6+6 padding + 11px text + 1px border top/bottom) plus the 4px
/// `Column` spacing above it and a small safety margin so the bar
/// isn't clipped in the densest layout (Skills with filter list at
/// the 260px cap, Chatting with the 480px chat panel).
pub const BAR_HEIGHT: f32 = 40.0;

pub fn view(mode: Mode, ctx: KeyhintContext) -> Element<'static, Message> {
    // When the floating options menu is open, replace the whole hint
    // set with menu-specific ones regardless of current mode.
    if ctx.session_menu_open {
        let mut bar: Row<'static, Message> =
            Row::new().spacing(12).align_y(iced::Alignment::Center);
        bar = bar.push(hint_item("esc", "Close"));
        bar = bar.push(hint_item("↑↓", "Navigate"));
        bar = bar.push(horizontal_space().width(Length::Fill));
        let action_label = match ctx.session_menu_selected {
            0 => {
                if ctx.selected_is_pinned {
                    "Unpin"
                } else {
                    "Pin"
                }
            }
            _ => "Archive",
        };
        bar = bar.push(hint_item("↵", action_label));
        return container(bar)
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
            .into();
    }

    let mut hints: Vec<(&'static str, &'static str)> = match mode {
        Mode::Idle if ctx.selection_active && ctx.has_rows => vec![
            ("↵", "Open"),
            ("↑↓", "Navigate"),
            ("esc", "Dismiss"),
        ],
        Mode::Idle if ctx.has_rows => vec![
            ("↵", "Send"),
            ("⌘↵", "Fire & forget"),
            ("↑↓", "Navigate"),
            ("esc", "Dismiss"),
        ],
        Mode::Idle => vec![
            ("↵", "Send"),
            ("⌘↵", "Fire & forget"),
            ("esc", "Dismiss"),
        ],
        Mode::Skills if ctx.skill_locked => vec![
            ("↵", "Run"),
            ("⌘↵", "Fire & forget"),
            ("esc", "Dismiss"),
        ],
        Mode::Skills => vec![("↵", "Select"), ("↑↓", "Navigate"), ("esc", "Dismiss")],
        Mode::ProjectPicker => {
            vec![("↵", "Switch"), ("↑↓", "Navigate"), ("esc", "Back")]
        }
        Mode::Chatting if ctx.is_generating && ctx.has_session_id => vec![
            ("↵", "Send"),
            ("⌘T", "Terminal"),
            ("esc", "Back"),
            ("ctrl c", "Cancel"),
        ],
        Mode::Chatting if ctx.is_generating => {
            vec![("↵", "Send"), ("esc", "Back"), ("ctrl c", "Cancel")]
        }
        Mode::Chatting if ctx.has_session_id => {
            vec![("↵", "Send"), ("⌘T", "Terminal"), ("esc", "Back")]
        }
        Mode::Chatting => vec![("↵", "Send"), ("esc", "Back")],
        Mode::Settings => vec![],
    };

    // Pin/unpin affordance: only meaningful inside a chat — pinning
    // snapshots the window's on-screen position *and* the active chat
    // id so the next summon restores both. Outside Chatting there's no
    // chat to pin, so the hint and shortcut are hidden.
    if matches!(mode, Mode::Chatting) {
        let label = if ctx.pinned { "Unpin" } else { "Pin" };
        hints.push(("⌘⇧P", label));
    }

    // Actions submenu affordance: only meaningful in Idle when a row is
    // selected and has a session id to tag. Hidden otherwise to avoid
    // advertising a shortcut that would be a no-op.
    if matches!(mode, Mode::Idle)
        && ctx.selection_active
        && ctx.has_rows
        && ctx.can_open_options
    {
        hints.push(("⌘K", "Actions"));
    }

    // Reorder affordance: swap the pinned row's position within the
    // pinned block. Only surfaced when the currently-selected idle row
    // is pinned — the shortcut is a no-op otherwise.
    if matches!(mode, Mode::Idle)
        && ctx.selection_active
        && ctx.has_rows
        && ctx.selected_is_pinned
    {
        hints.push(("⌘⇧↑↓", "Reorder"));
    }

    // Left cluster: `esc`, `⌘⇧P`, and `ctrl c` render flush-left in
    // that fixed order (so `⌘⇧P` always sits immediately right of
    // `esc`, regardless of each mode's original vec ordering).
    // Everything else flows flush-right preserving source order.
    const LEFT_ORDER: [&str; 3] = ["esc", "⌘⇧P", "ctrl c"];
    let (left_unordered, right): (Vec<_>, Vec<_>) = hints
        .into_iter()
        .partition(|(key, _)| LEFT_ORDER.iter().any(|k| k == key));
    let left: Vec<(&'static str, &'static str)> = LEFT_ORDER
        .iter()
        .filter_map(|desired| {
            left_unordered
                .iter()
                .find(|(k, _)| k == desired)
                .copied()
        })
        .collect();

    let mut bar: Row<'static, Message> = Row::new().spacing(12).align_y(iced::Alignment::Center);
    for (key, label) in left {
        bar = bar.push(hint_item(key, label));
    }
    bar = bar.push(horizontal_space().width(Length::Fill));
    if !ctx.project_path_display.is_empty() && !matches!(mode, Mode::Settings) {
        // Prefix the path with a ⌘P affordance so the user knows how
        // to change it. Suppressed inside the picker itself — the
        // user is already there, so the hint would just be noise.
        let mut center: Row<'static, Message> =
            Row::new().spacing(6).align_y(iced::Alignment::Center);
        if !matches!(mode, Mode::ProjectPicker) {
            center = center.push(kbd_chip("⌘P"));
        }
        center =
            center.push(text(ctx.project_path_display).size(11).color(theme::MUTED));
        bar = bar.push(center);
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
    row![kbd_chip(key), text(label).size(11).color(theme::MUTED)]
        .spacing(6)
        .align_y(iced::Alignment::Center)
        .into()
}

/// The boxed-key visual used in every key hint. Factored out so the
/// ⌘P chip beside the centered project path can reuse the same
/// styling without pulling a label along with it.
fn kbd_chip(key: &'static str) -> Element<'static, Message> {
    container(text(key).size(11).color(theme::TEXT))
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
        })
        .into()
}
