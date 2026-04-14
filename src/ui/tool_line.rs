//! Tool call UI: collapsible summary row + expanded per-tool rows.

use iced::widget::{button, container, horizontal_space, row, text};
use iced::{Element, Length, Padding};
use std::collections::BTreeMap;
use std::time::Duration;

use crate::app::Message;
use crate::state::{ChatMessageView, ContentBlock};

// ---------------------------------------------------------------------------
// Summary computation
// ---------------------------------------------------------------------------

pub struct ToolSummary {
    pub tool_call_count: usize,
    pub error_count: usize,
    pub files_changed: usize,
    pub duration: Option<Duration>,
}

fn is_file_mutating(tool: &str) -> bool {
    matches!(
        tool.to_ascii_lowercase().as_str(),
        "write" | "edit" | "create" | "multiedit"
    )
}

/// Count tool calls, errors, and file changes in a message's blocks.
pub fn count_tools(msg: &ChatMessageView) -> (usize, usize, usize) {
    let mut tool_call_count = 0usize;
    let mut error_count = 0usize;
    let mut files_changed = 0usize;
    for block in &msg.blocks {
        match block {
            ContentBlock::ToolEnd { tool, .. } => {
                tool_call_count += 1;
                if is_file_mutating(tool) {
                    files_changed += 1;
                }
            }
            ContentBlock::ToolStart { .. } => {
                tool_call_count += 1;
            }
            ContentBlock::Error(_) => {
                error_count += 1;
            }
            _ => {}
        }
    }
    (tool_call_count, error_count, files_changed)
}

/// Compute a summary for a completed message. Duration is passed in
/// externally (from `result_duration_ms` or local timing).
pub fn compute_summary(msg: &ChatMessageView, duration: Option<Duration>) -> ToolSummary {
    let (tool_call_count, error_count, files_changed) = count_tools(msg);
    ToolSummary {
        tool_call_count,
        error_count,
        files_changed,
        duration,
    }
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs >= 60 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}

// ---------------------------------------------------------------------------
// Streaming indicator (chat-level, with animated spinner)
// ---------------------------------------------------------------------------

const SPINNER_FRAMES: [&str; 4] = ["|", "/", "-", "\\"];

/// The "Doing stuff..." bar shown at the bottom of the chat panel during
/// generation. Includes a rotating ASCII spinner and live elapsed time.
pub fn summary_row_streaming<'a>(
    tool_call_count: usize,
    error_count: usize,
    files_changed: usize,
    elapsed: Duration,
    spinner_frame: u32,
) -> Element<'a, Message> {
    let glyph = SPINNER_FRAMES[(spinner_frame as usize) % SPINNER_FRAMES.len()];

    let dot_color = if error_count > 0 {
        super::theme::DANGER
    } else {
        super::theme::ACCENT
    };

    let mut r = row![].spacing(8).align_y(iced::Alignment::Center);

    // Spinner glyph in a fixed-width container so it doesn't jitter.
    r = r.push(
        container(text(glyph).size(12).color(dot_color))
            .width(Length::Fixed(10.0))
            .align_x(iced::Alignment::Center),
    );

    r = r.push(text("Doing stuff...").size(12).color(super::theme::TEXT));
    r = r.push(text("·").size(12).color(super::theme::MUTED));
    r = r.push(text(format_duration(elapsed)).size(12).color(super::theme::MUTED));

    if tool_call_count > 0 {
        r = r.push(text("·").size(12).color(super::theme::MUTED));
        let label = if tool_call_count == 1 {
            "1 tool call".to_string()
        } else {
            format!("{} tool calls", tool_call_count)
        };
        r = r.push(text(label).size(12).color(super::theme::MUTED));
    }

    if files_changed > 0 {
        r = r.push(text("·").size(12).color(super::theme::MUTED));
        let label = if files_changed == 1 {
            "1 file changed".to_string()
        } else {
            format!("{} files changed", files_changed)
        };
        r = r.push(text(label).size(12).color(super::theme::MUTED));
    }

    if error_count > 0 {
        r = r.push(text("·").size(12).color(super::theme::MUTED));
        let label = if error_count == 1 {
            "1 error".to_string()
        } else {
            format!("{} errors", error_count)
        };
        r = r.push(text(label).size(12).color(super::theme::DANGER));
    }

    container(r.width(Length::Fill))
        .padding(Padding::from([6.0, 10.0]))
        .width(Length::Fill)
        .style(|_theme: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(super::theme::SURFACE_2)),
            border: iced::Border {
                color: iced::Color::TRANSPARENT,
                width: 0.0,
                radius: 6.0.into(),
            },
            ..Default::default()
        })
        .into()
}

// ---------------------------------------------------------------------------
// Completed summary row (per-message, clickable expand/collapse)
// ---------------------------------------------------------------------------

pub fn summary_row<'a>(
    msg_id: u64,
    summary: &ToolSummary,
    expanded: bool,
) -> Element<'a, Message> {
    let dot_color = if summary.error_count > 0 {
        super::theme::DANGER
    } else {
        super::theme::ACCENT
    };

    let mut r = row![text("●").size(10).color(dot_color),]
        .spacing(8)
        .align_y(iced::Alignment::Center);

    r = r.push(text("Did stuff").size(12).color(super::theme::TEXT));

    if let Some(d) = summary.duration {
        r = r.push(text("·").size(12).color(super::theme::MUTED));
        r = r.push(text(format_duration(d)).size(12).color(super::theme::MUTED));
    }

    if summary.tool_call_count > 0 {
        r = r.push(text("·").size(12).color(super::theme::MUTED));
        let label = if summary.tool_call_count == 1 {
            "1 tool call".to_string()
        } else {
            format!("{} tool calls", summary.tool_call_count)
        };
        r = r.push(text(label).size(12).color(super::theme::MUTED));
    }

    if summary.files_changed > 0 {
        r = r.push(text("·").size(12).color(super::theme::MUTED));
        let label = if summary.files_changed == 1 {
            "1 file changed".to_string()
        } else {
            format!("{} files changed", summary.files_changed)
        };
        r = r.push(text(label).size(12).color(super::theme::MUTED));
    }

    if summary.error_count > 0 {
        r = r.push(text("·").size(12).color(super::theme::MUTED));
        let label = if summary.error_count == 1 {
            "1 error".to_string()
        } else {
            format!("{} errors", summary.error_count)
        };
        r = r.push(text(label).size(12).color(super::theme::DANGER));
    }

    r = r.push(horizontal_space());
    let chevron = if expanded { "▾" } else { "▸" };
    r = r.push(text(chevron).size(12).color(super::theme::MUTED));

    let inner = container(r.width(Length::Fill))
        .padding(Padding::from([6.0, 10.0]))
        .width(Length::Fill)
        .style(|_theme: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(super::theme::SURFACE_2)),
            border: iced::Border {
                color: iced::Color::TRANSPARENT,
                width: 0.0,
                radius: 6.0.into(),
            },
            ..Default::default()
        });

    button(inner)
        .on_press(Message::ToggleToolsExpanded(msg_id))
        .padding(0)
        .style(|_theme, _status| button::Style {
            background: None,
            text_color: super::theme::TEXT,
            border: iced::Border {
                color: iced::Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            shadow: iced::Shadow::default(),
        })
        .width(Length::Fill)
        .into()
}

// ---------------------------------------------------------------------------
// Expanded per-tool row
// ---------------------------------------------------------------------------

fn summarize_args(tool: &str, args: &BTreeMap<String, serde_json::Value>) -> String {
    let key = match tool.to_ascii_lowercase().as_str() {
        "bash" => "command",
        "glob" => "pattern",
        "grep" | "search" => "pattern",
        "read" => "file_path",
        "write" | "edit" | "create" | "multiedit" => "file_path",
        _ => {
            // Fallback: first short string value
            for v in args.values() {
                if let Some(s) = v.as_str() {
                    if s.len() < 80 {
                        return s.to_string();
                    }
                    return truncate(s, 80);
                }
            }
            return String::new();
        }
    };
    match args.get(key).and_then(|v| v.as_str()) {
        Some(s) => truncate(s, 80),
        None => String::new(),
    }
}

pub fn view_expanded(block: &ContentBlock) -> Element<'_, Message> {
    match block {
        ContentBlock::ToolStart { tool, args } => {
            // Running state: spinner-like dot, muted tool name.
            let summary = summarize_args(tool, args);

            let mut r = row![
                text("●").size(10).color(super::theme::ACCENT),
                text(tool.as_str())
                    .size(12)
                    .color(super::theme::MUTED),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center);

            if !summary.is_empty() {
                r = r.push(text(summary).size(11).color(super::theme::MUTED));
            }

            container(r)
                .padding(Padding::from(0.0).left(12.0))
                .into()
        }
        ContentBlock::ToolEnd { tool, args, result } => {
            // Completed state: checkmark, bold tool name.
            let summary = summarize_args(tool, args);

            let mut r = row![
                text("✓").size(12).color(super::theme::SUCCESS),
                text(tool.as_str())
                    .size(12)
                    .color(super::theme::TEXT)
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..Default::default()
                    }),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center);

            if !summary.is_empty() {
                r = r.push(text(summary).size(11).color(super::theme::MUTED));
            }

            // For file-mutating tools, try to show diff counts from the result.
            if is_file_mutating(tool) {
                if let Some(res) = result {
                    if let Some(diff) = parse_diff_counts(res) {
                        r = r.push(
                            text(diff)
                                .size(11)
                                .color(super::theme::SUCCESS),
                        );
                    }
                }
            }

            container(r)
                .padding(Padding::from(0.0).left(12.0))
                .into()
        }
        ContentBlock::Error(err) => {
            let r = row![
                text("✗").size(12).color(super::theme::DANGER),
                text(truncate(err, 120))
                    .size(12)
                    .color(super::theme::DANGER),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center);

            container(r)
                .padding(Padding::from(0.0).left(12.0))
                .into()
        }
        ContentBlock::Text { .. } => container(text("")).width(Length::Shrink).into(),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push_str("...");
        out
    }
}

/// Try to extract `+N -M` diff counts from a tool result string.
fn parse_diff_counts(result: &str) -> Option<String> {
    let mut additions = None;
    let mut deletions = None;
    for word in result.split_whitespace() {
        if let Some(rest) = word.strip_prefix('+') {
            if let Ok(n) = rest.parse::<usize>() {
                additions = Some(n);
            }
        } else if let Some(rest) = word.strip_prefix('-') {
            if let Ok(n) = rest.parse::<usize>() {
                deletions = Some(n);
            }
        }
    }
    match (additions, deletions) {
        (Some(a), Some(d)) => Some(format!("+{} -{}", a, d)),
        (Some(a), None) => Some(format!("+{}", a)),
        _ => None,
    }
}
