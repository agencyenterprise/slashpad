//! Streaming chat panel.

use iced::widget::{container, markdown, row, scrollable, text, Column};
use iced::{Element, Length};

use crate::app::Message;
use crate::state::{ChatMessageView, ContentBlock, MessageStatus, Role};

pub fn view<'a>(
    messages: &'a [ChatMessageView],
    is_agent_ready: bool,
    spinner_frame: u32,
) -> Element<'a, Message> {
    let mut col: Column<'a, Message> = Column::new().spacing(12);

    for msg in messages {
        match msg.role {
            Role::User => col = col.push(user_bubble(msg)),
            Role::Assistant => col = col.push(assistant_bubble(msg)),
        }
    }

    // Show the animated "Working…" indicator from Submit until the turn
    // finishes. Covers both the pre-stream window (last message is the
    // user's prompt) and the streaming window (last message is a partial
    // assistant bubble).
    let is_generating = !is_agent_ready
        && messages.last().is_some_and(|m| {
            m.role == Role::User
                || (m.role == Role::Assistant && m.status == MessageStatus::Streaming)
        });
    if is_generating {
        col = col.push(spinner_row(spinner_frame));
    }

    container(
        scrollable(container(col).padding([0, 14]))
            .height(Length::Fill)
            .direction(super::theme::scrollbar_direction())
            .style(super::theme::scrollbar_style),
    )
        .padding([14, 0])
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
            ContentBlock::Text { parsed, .. } => {
                // Render via iced's built-in markdown widget, which
                // handles headings, paragraphs, lists, code blocks,
                // inline formatting, and links. Items are pre-parsed
                // on every text_delta inside `app.rs`.
                let element = markdown::view(
                    parsed,
                    markdown::Settings::with_text_size(13),
                    markdown::Style::from_palette(super::theme::dark_theme().palette()),
                )
                .map(Message::MarkdownLinkClicked);
                col = col.push(element);
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

/// Animated spinner + "Working…" label. Cycles the glyph on each
/// `SpinnerTick` so the indicator reads as live while a turn is in
/// flight. Uses ASCII-only spinner characters (`| / - \`) because
/// iced's default font lacks glyphs for the Unicode Braille block and
/// most other "fancy" spinner sets, which render as tofu. The glyph is
/// pinned inside a fixed-width container so the label doesn't shift as
/// the frame changes (the ASCII glyphs have different widths in the
/// default proportional font). Colors match the existing theme —
/// ACCENT for the glyph, MUTED for the label.
fn spinner_row<'a>(frame: u32) -> Element<'a, Message> {
    const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
    let glyph = FRAMES[(frame as usize) % FRAMES.len()];
    row![
        container(text(glyph).size(14).color(super::theme::ACCENT))
            .width(Length::Fixed(12.0))
            .align_x(iced::Alignment::Center),
        text("Working…").size(13).color(super::theme::MUTED),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center)
    .into()
}
