//! Session history — wraps one-shot sidecar runs in `list` and `messages`
//! modes to fetch recent sessions and past-session messages.

use std::path::Path;

use crate::sidecar::{self, FollowUp, Payload, SidecarEvent};
use crate::state::{ChatMessageView, ContentBlock, MessageStatus, Role, SessionInfo};

/// Collapse the Claude Agent SDK's slash-command envelope back to the
/// human form the user originally typed. The SDK expands any prompt
/// starting with `/` into three newline-separated tags
/// (`<command-message>`, `<command-name>`, `<command-args>`) before
/// writing it to the session JSONL, so resumed chats see the XML
/// verbatim. New chats don't hit this because we render user bubbles
/// from the raw input in `ChatState` — this only touches the
/// `messages` load path. If the envelope isn't present, leave the text
/// alone.
fn unwrap_slash_command(content: &str) -> String {
    let trimmed = content.trim();
    let name = extract_tag(trimmed, "command-name");
    let args = extract_tag(trimmed, "command-args");
    match (name, args) {
        (Some(n), Some(a)) if !n.is_empty() => {
            if a.is_empty() {
                n
            } else {
                format!("{n} {a}")
            }
        }
        _ => content.to_string(),
    }
}

fn extract_tag(s: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = s.find(&open)? + open.len();
    let end = s[start..].find(&close)? + start;
    Some(s[start..end].trim().to_string())
}

/// Spawn `runner.mjs list` against `cwd` and collect recent sessions.
/// Sessions are scoped per-`cwd` because that's what the Claude Code
/// CLI uses to key its `~/.claude/projects/` subdirectories; passing
/// the current project path means the idle list reflects whatever
/// project the user has selected via Cmd+P.
pub async fn list_recent(cwd: &Path) -> anyhow::Result<Vec<SessionInfo>> {
    let payload = Payload::list(cwd.to_string_lossy().to_string());
    let mut spawned = sidecar::spawn(payload)?;

    let mut out = Vec::new();
    while let Some(event) = spawned.event_rx.recv().await {
        match event {
            SidecarEvent::Session {
                session_id,
                summary,
                last_modified,
                first_prompt,
            } => out.push(SessionInfo {
                session_id,
                summary,
                last_modified,
                first_prompt,
            }),
            SidecarEvent::Complete { .. } | SidecarEvent::Error { .. } => break,
            _ => {}
        }
    }
    let _ = spawned.follow_up_tx.send(FollowUp::Close);
    Ok(out)
}

/// Spawn `runner.mjs messages <sessionId>` against `cwd` and collect
/// historical chat messages. `cwd` must be the same project the
/// session was recorded in — sessions live under
/// `~/.claude/projects/<mangled-cwd>/` so the runner needs it to
/// locate the JSONL.
pub async fn load_messages(cwd: &Path, session_id: &str) -> anyhow::Result<Vec<ChatMessageView>> {
    let payload = Payload::messages(session_id.to_string(), cwd.to_string_lossy().to_string());
    let mut spawned = sidecar::spawn(payload)?;

    let mut out: Vec<ChatMessageView> = Vec::new();
    let mut next_id: u64 = 1;
    while let Some(event) = spawned.event_rx.recv().await {
        match event {
            SidecarEvent::ChatMessage {
                role,
                content,
                tool_events,
                duration_ms,
                ..
            } => {
                let role = if role == "user" {
                    Role::User
                } else {
                    Role::Assistant
                };

                // Collect blocks from this event.
                let mut blocks: Vec<ContentBlock> = Vec::new();
                if let Some(events) = tool_events {
                    for ev in events {
                        match ev {
                            SidecarEvent::ToolStart {
                                tool,
                                tool_use_id,
                                args,
                                ..
                            } => {
                                blocks.push(ContentBlock::ToolStart {
                                    tool,
                                    tool_use_id,
                                    args: args.unwrap_or_default(),
                                });
                            }
                            SidecarEvent::ToolEnd {
                                tool,
                                tool_use_id,
                                args,
                                result,
                                ..
                            } => {
                                // Replace matching ToolStart, same as the
                                // streaming path in ChatState::apply_event.
                                let tool_use_id_ref = tool_use_id.clone();
                                let replaced = blocks.iter_mut().rev().find(|b| match b {
                                    ContentBlock::ToolStart {
                                        tool_use_id: id,
                                        tool: t,
                                        ..
                                    } => match (id, &tool_use_id_ref) {
                                        (Some(a), Some(b)) => a == b,
                                        _ => t == &tool,
                                    },
                                    _ => false,
                                });
                                if let Some(slot) = replaced {
                                    *slot = ContentBlock::ToolEnd {
                                        tool,
                                        tool_use_id,
                                        args: args.unwrap_or_default(),
                                        result,
                                        is_error: false,
                                    };
                                } else {
                                    blocks.push(ContentBlock::ToolEnd {
                                        tool,
                                        tool_use_id,
                                        args: args.unwrap_or_default(),
                                        result,
                                        is_error: false,
                                    });
                                }
                            }
                            SidecarEvent::ToolResult {
                                tool_use_id,
                                content,
                                is_error,
                                ..
                            } => {
                                // Patch the previously-appended ToolEnd with
                                // the real result text + error flag from the
                                // historical user message.
                                let patched =
                                    blocks.iter_mut().rev().find_map(|b| match b {
                                        ContentBlock::ToolEnd {
                                            tool_use_id: Some(id),
                                            ..
                                        } if id == &tool_use_id => Some(b),
                                        _ => None,
                                    });
                                if let Some(ContentBlock::ToolEnd {
                                    result,
                                    is_error: err_slot,
                                    ..
                                }) = patched
                                {
                                    if content.is_some() {
                                        *result = content;
                                    }
                                    *err_slot = is_error;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                if let Some(text) = content.as_ref() {
                    if !text.is_empty() {
                        let cleaned = if role == Role::User {
                            unwrap_slash_command(text)
                        } else {
                            text.clone()
                        };
                        blocks.push(ContentBlock::text(cleaned));
                    }
                }

                // Merge consecutive assistant messages into one so all
                // tool calls from a multi-turn agent run appear under a
                // single "Did stuff" summary.
                let merged = role == Role::Assistant
                    && out.last().is_some_and(|m| m.role == Role::Assistant);

                if merged {
                    if let Some(prev) = out.last_mut() {
                        prev.blocks.extend(blocks);
                        // Use the latest duration — each consecutive assistant
                        // message's duration is measured from the same user
                        // timestamp, so the last one covers the full turn.
                        if duration_ms.is_some() {
                            prev.result_duration_ms = duration_ms;
                        }
                    }
                } else {
                    out.push(ChatMessageView {
                        id: next_id,
                        role,
                        blocks,
                        status: MessageStatus::Complete,
                        tools_expanded: false,
                        result_duration_ms: duration_ms,
                    });
                    next_id += 1;
                }
            }
            SidecarEvent::Complete { .. } | SidecarEvent::Error { .. } => break,
            _ => {}
        }
    }
    let _ = spawned.follow_up_tx.send(FollowUp::Close);
    Ok(out)
}
