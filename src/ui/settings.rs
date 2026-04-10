//! Settings panel — API key + hotkey display.

use iced::widget::{button, column, container, row, text, text_input};
use iced::{Element, Length};

use crate::app::Message;

pub fn view<'a>(
    api_key_input: &'a str,
    current_hotkey: &'a str,
    _recording: bool,
) -> Element<'a, Message> {
    let header = row![
        text("Settings").size(13).color(super::theme::TEXT),
        iced::widget::horizontal_space(),
        button(text("esc").size(11).color(super::theme::MUTED))
            .on_press(Message::CloseSettings)
            .padding(0)
            .style(|_, _| iced::widget::button::Style {
                background: None,
                text_color: super::theme::MUTED,
                ..Default::default()
            }),
    ]
    .align_y(iced::Alignment::Center);

    let api_row = column![
        text("Anthropic API Key")
            .size(11)
            .color(super::theme::MUTED),
        row![
            text_input("sk-ant-...", api_key_input)
                .on_input(Message::ApiKeyInputChanged)
                .padding(8)
                .size(13)
                .width(Length::Fill),
            button(text("Save").size(12).color(super::theme::SURFACE_0))
                .on_press(Message::SaveApiKey)
                .padding([8, 14])
                .style(|_, _| iced::widget::button::Style {
                    background: Some(iced::Background::Color(super::theme::ACCENT)),
                    text_color: super::theme::SURFACE_0,
                    border: iced::Border {
                        color: iced::Color::TRANSPARENT,
                        width: 0.0,
                        radius: 8.0.into(),
                    },
                    ..Default::default()
                }),
        ]
        .spacing(8),
        text("Or run `claude login` in your terminal")
            .size(11)
            .color(super::theme::MUTED),
    ]
    .spacing(6);

    let hotkey_row = column![
        text("Global Hotkey").size(11).color(super::theme::MUTED),
        text(current_hotkey.to_string())
            .size(13)
            .color(super::theme::TEXT),
    ]
    .spacing(6);

    let body = column![header, api_row, hotkey_row].spacing(14).padding(16);

    container(body)
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
