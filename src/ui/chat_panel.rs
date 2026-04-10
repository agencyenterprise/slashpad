//! Streaming chat panel.

use iced::widget::{container, scrollable, text, Column};
use iced::{Element, Length};

use crate::app::Message;
use crate::markdown;
use crate::state::{ChatMessageView, ContentBlock, MessageStatus, Role};

pub fn view<'a>(messages: &'a [ChatMessageView], is_agent_ready: bool) -> Element<'a, Message> {
    let mut col: Column<'a, Message> = Column::new().spacing(12);

    for msg in messages {
        match msg.role {
            Role::User => col = col.push(user_bubble(msg)),
            Role::Assistant => col = col.push(assistant_bubble(msg)),
        }
    }

    if !is_agent_ready
        && messages
            .last()
            .is_some_and(|m| m.role == Role::Assistant && m.status == MessageStatus::Streaming)
    {
        col = col.push(
            text("Thinking...")
                .size(12)
                .color(super::theme::MUTED),
        );
    }

    container(scrollable(col).height(Length::Fill))
        .padding(14)
        .width(Length::Fill)
        .height(Length::Fixed(480.0))
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

fn user_bubble<'a>(msg: &'a ChatMessageView) -> Element<'a, Message> {
    let content = msg.flat_text();
    container(text(content).size(13).color(super::theme::TEXT))
        .padding([8, 14])
        .style(|_theme: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(super::theme::SURFACE_3)),
            border: iced::Border {
                color: iced::Color::TRANSPARENT,
                width: 0.0,
                radius: 10.0.into(),
            },
            text_color: Some(super::theme::TEXT),
            ..Default::default()
        })
        .into()
}

fn assistant_bubble<'a>(msg: &'a ChatMessageView) -> Element<'a, Message> {
    let mut col: Column<'a, Message> = Column::new().spacing(6);
    for block in &msg.blocks {
        match block {
            ContentBlock::Text(t) => {
                let rendered = markdown::render_plain(t);
                col = col.push(
                    text(rendered)
                        .size(13)
                        .color(super::theme::TEXT),
                );
            }
            ContentBlock::ToolStart { .. }
            | ContentBlock::ToolEnd { .. }
            | ContentBlock::Error(_) => {
                col = col.push(super::tool_line::view(block));
            }
        }
    }
    col.into()
}
