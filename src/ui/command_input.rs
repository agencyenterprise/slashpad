//! Top command input bar.

use iced::widget::{container, text_input};
use iced::{Element, Length};

use crate::app::{Message, INPUT_ID};
use crate::state::Mode;

pub fn view(value: &str, mode: Mode, is_agent_ready: bool) -> Element<'_, Message> {
    let placeholder = match mode {
        Mode::Chatting if is_agent_ready => "Send a follow-up...",
        Mode::Chatting => "Waiting for response...",
        Mode::ProjectPicker => "Search for a project...",
        _ => "Run a command or ask anything...",
    };

    let input = text_input(placeholder, value)
        .id(INPUT_ID.clone())
        .on_input(Message::InputChanged)
        .on_submit(Message::Submit)
        .size(15)
        .padding(12)
        .width(Length::Fill)
        .style(|_theme: &iced::Theme, _status| iced::widget::text_input::Style {
            background: iced::Background::Color(iced::Color::TRANSPARENT),
            border: iced::Border {
                color: iced::Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            icon: super::theme::MUTED,
            placeholder: super::theme::MUTED,
            value: super::theme::TEXT,
            selection: iced::Color {
                a: 0.35,
                ..super::theme::ACCENT
            },
        });

    container(input)
        .padding(12)
        .width(Length::Fill)
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
