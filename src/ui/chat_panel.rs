//! Streaming chat panel.

use iced::widget::{container, markdown, scrollable, text, Column};
use iced::{Element, Length};
use std::time::Instant;

use crate::app::Message;
use crate::state::{ChatMessageView, ContentBlock, MessageStatus, Role};

/// Extra context passed from app.rs for the streaming indicator.
pub struct StreamingContext {
    pub turn_submitted_at: Option<Instant>,
    pub spinner_frame: u32,
}

pub fn view<'a>(
    messages: &'a [ChatMessageView],
    is_generating: bool,
    turn_submitted_at: Option<Instant>,
    spinner_frame: u32,
    scroll_id: scrollable::Id,
) -> Element<'a, Message> {
    let mut col: Column<'a, Message> = Column::new().spacing(12).width(Length::Fill);

    let streaming_ctx = if is_generating {
        Some(StreamingContext {
            turn_submitted_at,
            spinner_frame,
        })
    } else {
        None
    };

    for msg in messages {
        match msg.role {
            Role::User => col = col.push(user_bubble(msg)),
            Role::Assistant => col = col.push(assistant_bubble(msg, &streaming_ctx)),
        }
    }

    // If generating but no assistant message exists yet (pre-stream
    // window between submit and first event), show a standalone
    // "Working..." bar. Only when the last message is a user bubble
    // (no assistant response at all yet).
    if is_generating {
        let awaiting_response = messages
            .last()
            .is_some_and(|m| m.role == Role::User);

        if awaiting_response {
            if let Some(started) = turn_submitted_at {
                col = col.push(super::tool_line::summary_row_streaming(
                    0, 0, 0,
                    started.elapsed(),
                    spinner_frame,
                ));
            }
        }
    }

    container(
        scrollable(container(col).padding([0, 14]).width(Length::Fill))
            .id(scroll_id)
            .on_scroll(Message::ChatScrolled)
            .width(Length::Fill)
            .height(Length::Fill)
            .direction(super::theme::scrollbar_direction())
            .style(super::theme::scrollbar_style),
    )
        .padding([14, 0])
        .width(Length::Fill)
        .height(Length::Fixed(480.0))
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

fn has_tool_blocks(msg: &ChatMessageView) -> bool {
    msg.blocks.iter().any(|b| {
        matches!(
            b,
            ContentBlock::ToolStart { .. } | ContentBlock::ToolEnd { .. } | ContentBlock::Error(_)
        )
    })
}

fn is_tool_block(block: &ContentBlock) -> bool {
    matches!(
        block,
        ContentBlock::ToolStart { .. } | ContentBlock::ToolEnd { .. } | ContentBlock::Error(_)
    )
}

fn assistant_bubble<'a>(
    msg: &'a ChatMessageView,
    streaming_ctx: &Option<StreamingContext>,
) -> Element<'a, Message> {
    let mut col: Column<'a, Message> = Column::new().spacing(12);
    let has_tools = has_tool_blocks(msg);

    if msg.status == MessageStatus::Streaming {
        // Streaming: "Working..." bar at the top with tool calls nested
        // under it, then text blocks below.
        if let Some(ctx) = streaming_ctx {
            if let Some(started) = ctx.turn_submitted_at {
                let (tool_count, error_count, files_changed) =
                    super::tool_line::count_tools(msg);

                col = col.push(super::tool_line::summary_row_streaming(
                    tool_count,
                    error_count,
                    files_changed,
                    started.elapsed(),
                    ctx.spinner_frame,
                ));
            }
        }

        // Tool calls appear indented under the Working bar.
        for block in &msg.blocks {
            if is_tool_block(block) {
                col = col.push(super::tool_line::view_expanded(block));
            } else if let ContentBlock::Text { parsed, .. } = block {
                col = col.push(render_markdown(parsed));
            }
        }
    } else if has_tools && !msg.tools_expanded {
        // Complete, collapsed: single summary row replacing all tool blocks.
        let duration = msg
            .result_duration_ms
            .map(std::time::Duration::from_millis);
        let summary = super::tool_line::compute_summary(msg, duration);
        let mut summary_emitted = false;

        for block in &msg.blocks {
            if is_tool_block(block) {
                if !summary_emitted {
                    col = col.push(super::tool_line::summary_row(
                        msg.id, &summary, false,
                    ));
                    summary_emitted = true;
                }
            } else if let ContentBlock::Text { parsed, .. } = block {
                col = col.push(render_markdown(parsed));
            }
        }
    } else if has_tools && msg.tools_expanded {
        // Complete, expanded: summary row + individual tool rows.
        let duration = msg
            .result_duration_ms
            .map(std::time::Duration::from_millis);
        let summary = super::tool_line::compute_summary(msg, duration);
        let mut summary_emitted = false;

        for block in &msg.blocks {
            if is_tool_block(block) {
                if !summary_emitted {
                    col = col.push(super::tool_line::summary_row(
                        msg.id, &summary, true,
                    ));
                    summary_emitted = true;
                }
                col = col.push(super::tool_line::view_expanded(block));
            } else if let ContentBlock::Text { parsed, .. } = block {
                col = col.push(render_markdown(parsed));
            }
        }
    } else {
        // No tool blocks — just render text.
        for block in &msg.blocks {
            if let ContentBlock::Text { parsed, .. } = block {
                col = col.push(render_markdown(parsed));
            }
        }
    }

    col.into()
}

fn render_markdown<'a>(
    parsed: &'a [iced::widget::markdown::Item],
) -> Element<'a, Message> {
    markdown::view(
        parsed,
        markdown::Settings::with_text_size(13),
        markdown::Style::from_palette(super::theme::dark_theme().palette()),
    )
    .map(Message::MarkdownLinkClicked)
}
