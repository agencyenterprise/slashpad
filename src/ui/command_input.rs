//! Top command input bar.

use iced::widget::{container, text_input};
use iced::{Element, Length};

use crate::app::{Message, Slashpad, INPUT_ID};
use crate::state::Mode;
use crate::ui::shortcut_filter::ShortcutFilter;

pub fn view(value: &str, mode: Mode, is_agent_ready: bool) -> Element<'_, Message> {
    let placeholder = match mode {
        Mode::Chatting if is_agent_ready => "Send a follow-up...",
        Mode::Chatting => "Wait for Claude or interrupt",
        Mode::ProjectPicker => "Search for a project...",
        _ => "Search, type / for skills, or prompt Claude Code",
    };

    let input = text_input(placeholder, value)
        .id(INPUT_ID.clone())
        .on_input(Message::InputChanged)
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

    // Drop only the modifier+letter events that map to app shortcuts,
    // so the shortcut's letter never gets inserted into text_input's
    // internal buffer (which otherwise produces a one-frame visible
    // flash before our update handler can strip the leaked char).
    // Unbound Cmd+letter combos (Cmd+C/V/X/A) are allowed through so
    // clipboard and select-all keep working.
    let input = ShortcutFilter::new(input, Slashpad::should_filter_launcher_keypress);

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
