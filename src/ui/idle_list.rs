//! Unified idle-list: active chats + recent past sessions.
//!
//! Rows are built by the caller (`Launchpad::build_idle_rows_view`) as
//! borrowed references into `self.chats` / `self.recent_sessions`, so
//! the view lifetime is tied to `&Launchpad`.

use iced::widget::{button, container, row, text, Column};
use iced::{Element, Length};

use crate::app::{ChatEntry, Message};
use crate::state::{ChatStatus, SessionInfo};

/// One row in the unified idle list.
pub enum IdleRow<'a> {
    /// A chat running (or completed) in this process.
    Active(&'a ChatEntry),
    /// A session loaded from disk that isn't also open as an active chat.
    Past(&'a SessionInfo),
}

pub fn view<'a>(
    rows: &[IdleRow<'a>],
    selected: usize,
    spinner_frame: u32,
) -> Element<'a, Message> {
    let mut col: Column<'a, Message> = Column::new();

    // Track the filtered-past-session index separately: callers of
    // `Message::SelectSession` must pass an index into the past-rows
    // list, not the unified list, to stay in sync with
    // `Launchpad::past_session_rows()`.
    let mut past_index: usize = 0;

    for (i, row_item) in rows.iter().enumerate() {
        let is_selected = i == selected;
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

    container(col)
        .padding(0)
        .width(Length::Fill)
        .style(|_theme: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(super::theme::SURFACE_1)),
            border: iced::Border {
                color: super::theme::SURFACE_3,
                width: 1.0,
                radius: 12.0.into(),
            },
            text_color: Some(super::theme::TEXT),
            ..Default::default()
        })
        .into()
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
