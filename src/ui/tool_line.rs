//! Single tool call row (used inside ChatPanel).

use iced::widget::{column, container, row, text, Column};
use iced::{Element, Length, Padding};

use crate::app::Message;
use crate::state::ContentBlock;

fn args_padding() -> Padding {
    Padding::from(0.0).top(4.0).left(24.0)
}

pub fn view(block: &ContentBlock) -> Element<'_, Message> {
    match block {
        ContentBlock::ToolStart { tool, args } => {
            let args_text = if args.is_empty() {
                String::new()
            } else {
                serde_json::to_string_pretty(args).unwrap_or_default()
            };
            let mut col: Column<'_, Message> = column![row![
                text("●").size(10).color(super::theme::ACCENT),
                text(format!("Running {}...", tool))
                    .size(12)
                    .color(super::theme::MUTED),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center)];
            if !args_text.is_empty() {
                col = col.push(
                    container(text(args_text).size(11).color(super::theme::MUTED))
                        .padding(args_padding()),
                );
            }
            col.into()
        }
        ContentBlock::ToolEnd { tool, args, result } => {
            let args_text = if args.is_empty() {
                String::new()
            } else {
                serde_json::to_string_pretty(args).unwrap_or_default()
            };
            let head = if let Some(r) = result {
                text(format!("✓ {} — {}", tool, truncate(r, 120)))
            } else {
                text(format!("✓ {}", tool))
            }
            .size(12)
            .color(super::theme::SUCCESS);
            let mut col: Column<'_, Message> = column![head];
            if !args_text.is_empty() {
                col = col.push(
                    container(text(args_text).size(11).color(super::theme::MUTED))
                        .padding(args_padding()),
                );
            }
            col.into()
        }
        ContentBlock::Error(err) => text(format!("✗ {err}"))
            .size(12)
            .color(super::theme::DANGER)
            .into(),
        ContentBlock::Text(_) => container(text("")).width(Length::Shrink).into(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
