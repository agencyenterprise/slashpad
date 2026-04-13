//! Unified idle-list: active chats + recent past sessions.
//!
//! Rows are built by the caller (`Launchpad::build_idle_rows_view`) as
//! borrowed references into `self.chats` / `self.recent_sessions`, so
//! the view lifetime is tied to `&Launchpad`.

use iced::widget::{button, container, row, scrollable, text, Column};
use iced::{Element, Length};

use crate::app::{ChatEntry, Message};
use crate::state::{ChatStatus, SessionInfo};

/// One row in the unified idle list.
pub enum IdleRow<'a> {
    /// A chat running (or completed) in this process.
    Active(&'a ChatEntry),
    /// A session loaded from disk that isn't also open as an active
    /// chat. Owned because the caller builds a fuzzy-filtered Vec —
    /// borrowing from that Vec's lifetime would escape the view fn.
    Past(SessionInfo),
}

pub fn view<'a>(
    rows: Vec<IdleRow<'a>>,
    selected: usize,
    spinner_frame: u32,
    scroll_id: scrollable::Id,
) -> Element<'a, Message> {
    let mut col: Column<'a, Message> = Column::new();

    // Track the filtered-past-session index separately: callers of
    // `Message::SelectSession` must pass an index into the past-rows
    // list, not the unified list, to stay in sync with
    // `Launchpad::past_session_rows()`.
    let mut past_index: usize = 0;
    let last = rows.len().saturating_sub(1);

    for (i, row_item) in rows.into_iter().enumerate() {
        let is_selected = i == selected;
        let is_first = i == 0;
        let is_last = i == last;
        let (row_el, msg) = match row_item {
            IdleRow::Active(entry) => {
                let title = entry.state.title.clone();
                let status_label = status_text(entry.state.status, spinner_frame);
                let status_color = status_color(entry.state.status);
                let row = row![
                    text(title)
                        .size(13)
                        .color(super::theme::TEXT)
                        .width(Length::FillPortion(4)),
                    text(status_label)
                        .size(11)
                        .color(status_color)
                        .width(Length::Shrink),
                ]
                .spacing(12)
                .align_y(iced::Alignment::Center);
                (row, Message::SelectChat(entry.state.id))
            }
            IdleRow::Past(session) => {
                let time = format_relative(session.last_modified);
                let row = row![
                    text(session.summary.clone())
                        .size(13)
                        .color(super::theme::TEXT)
                        .width(Length::FillPortion(4)),
                    text(time)
                        .size(11)
                        .color(super::theme::MUTED)
                        .width(Length::Shrink),
                ]
                .spacing(12)
                .align_y(iced::Alignment::Center);
                let idx = past_index;
                past_index += 1;
                (row, Message::SelectSession(idx))
            }
        };

        let row_container = container(row_el).padding([10, 18]).width(Length::Fill).style(
            move |_theme: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(if is_selected {
                    super::theme::SURFACE_3
                } else {
                    iced::Color::TRANSPARENT
                })),
                border: iced::Border {
                    color: iced::Color::TRANSPARENT,
                    width: 0.0,
                    radius: selection_radius(is_first, is_last),
                },
                text_color: Some(super::theme::TEXT),
                ..Default::default()
            },
        );

        let btn = button(row_container)
            .on_press(msg)
            .padding(0)
            .width(Length::Fill)
            .style(|_theme, _status| iced::widget::button::Style {
                background: None,
                text_color: super::theme::TEXT,
                ..Default::default()
            });
        col = col.push(btn);
    }

    // See `skill_list::view` for the `max_height` rationale.
    container(
        scrollable(col)
            .id(scroll_id)
            .direction(super::theme::scrollbar_direction())
            .style(super::theme::scrollbar_style),
    )
        .padding([4, 0])
        .width(Length::Fill)
        .max_height(260.0)
        .style(|_theme: &iced::Theme| iced::widget::container::Style {
            background: None,
            border: iced::Border {
                color: iced::Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            text_color: Some(super::theme::TEXT),
            ..Default::default()
        })
        .into()
}

/// Per-row corner radii. The panel no longer has its own rounded frame
/// (the unified outer container in `app.rs::view()` provides that), so
/// every row draws square selection corners flush with the section
/// dividers above and below.
fn selection_radius(_is_first: bool, _is_last: bool) -> iced::border::Radius {
    0.0.into()
}

fn status_text(status: ChatStatus, spinner_frame: u32) -> String {
    const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
    let glyph = FRAMES[(spinner_frame as usize) % FRAMES.len()];
    match status {
        ChatStatus::Initializing => format!("{glyph} starting"),
        ChatStatus::Streaming => format!("{glyph} streaming"),
        ChatStatus::Idle => "ready".to_string(),
        ChatStatus::Error => "error".to_string(),
        ChatStatus::Closed => "closed".to_string(),
    }
}

fn status_color(status: ChatStatus) -> iced::Color {
    match status {
        ChatStatus::Initializing | ChatStatus::Streaming => super::theme::ACCENT,
        ChatStatus::Idle => super::theme::SUCCESS,
        ChatStatus::Error => super::theme::DANGER,
        ChatStatus::Closed => super::theme::MUTED,
    }
}

fn format_relative(unix_millis: i64) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let diff_ms = now - unix_millis;
    if diff_ms < 0 {
        return "just now".to_string();
    }
    let minutes = diff_ms / 60_000;
    if minutes < 1 {
        "just now".to_string()
    } else if minutes < 60 {
        format!("{minutes}m ago")
    } else {
        let hours = minutes / 60;
        if hours < 24 {
            format!("{hours}h ago")
        } else {
            let days = hours / 24;
            format!("{days}d ago")
        }
    }
}
