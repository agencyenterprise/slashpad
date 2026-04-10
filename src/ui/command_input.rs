//! Top command input bar.

use iced::widget::{container, row, text, text_input};
use iced::{Element, Length};

use crate::app::{Message, INPUT_ID};
use crate::state::Mode;

pub fn view(value: &str, mode: Mode, is_agent_ready: bool) -> Element<'_, Message> {
    let placeholder = match mode {
        Mode::Chatting if is_agent_ready => "Send a follow-up...",
        Mode::Chatting => "Waiting for response...",
        _ => "Run a command or ask anything...",
    };

    let input = text_input(placeholder, value)
        .id(INPUT_ID.clone())
        .on_input(Message::InputChanged)
        .on_submit(Message::Submit)
        .size(15)
        .padding(12)
        .width(Length::Fill);

    let hint = match mode {
        Mode::Chatting if is_agent_ready && !value.is_empty() => "↵ reply",
        Mode::Skills => "↵ run",
        _ if value.starts_with('/') => "↵ run",
        _ if !value.is_empty() => "↵ send",
        _ => "/ skills",
    };

    let row = row![
        input,
        container(text(hint).size(11).color(super::theme::MUTED))
            .padding([6, 10])
            .width(Length::Shrink),
        container(text("esc").size(11).color(super::theme::MUTED))
            .padding([6, 10])
            .width(Length::Shrink),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    container(row)
        .padding(12)
        .width(Length::Fill)
        .style(|_theme: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(super::theme::SURFACE_1)),
            border: iced::Border {
                color: super::theme::SURFACE_3,
                width: 1.0,
                radius: 16.0.into(),
            },
            text_color: Some(super::theme::TEXT),
            ..Default::default()
        })
        .into()
}
