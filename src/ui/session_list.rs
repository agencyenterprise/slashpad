//! Recent sessions (shown in idle mode).

use iced::widget::{button, container, row, text, Column};
use iced::{Element, Length};

use crate::app::Message;
use crate::state::SessionInfo;

pub fn view(sessions: &[SessionInfo], selected: usize) -> Element<'_, Message> {
    let mut col: Column<'_, Message> = Column::new();
    for (i, session) in sessions.iter().enumerate() {
        let is_selected = i == selected;
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

        let row_container = container(row).padding([10, 18]).width(Length::Fill).style(
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
            .on_press(Message::SelectSession(i))
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
