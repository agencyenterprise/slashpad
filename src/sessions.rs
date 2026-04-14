//! Session history — wraps one-shot sidecar runs in `list` and `messages`
//! modes to fetch recent sessions and past-session messages.

use std::path::Path;

use crate::sidecar::{self, FollowUp, Payload, SidecarEvent};
use crate::state::{ChatMessageView, ContentBlock, MessageStatus, Role, SessionInfo};

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

    let mut out = Vec::new();
    let mut next_id: u64 = 1;
    while let Some(event) = spawned.event_rx.recv().await {
        match event {
            SidecarEvent::ChatMessage {
                role,
                content,
                tool_events,
                ..
            } => {
                let role = if role == "user" {
                    Role::User
                } else {
                    Role::Assistant
                };
                let mut blocks: Vec<ContentBlock> = Vec::new();
                if let Some(text) = content.as_ref() {
                    if !text.is_empty() {
                        blocks.push(ContentBlock::text(text.clone()));
                    }
                }
                if let Some(events) = tool_events {
                    for ev in events {
                        match ev {
                            SidecarEvent::ToolStart { tool, args, .. } => {
                                blocks.push(ContentBlock::ToolStart {
                                    tool,
                                    args: args.unwrap_or_default(),
                                });
                            }
                            SidecarEvent::ToolEnd {
                                tool,
                                args,
                                result,
                                ..
                            } => {
                                blocks.push(ContentBlock::ToolEnd {
                                    tool,
                                    args: args.unwrap_or_default(),
                                    result,
                                });
                            }
                            _ => {}
                        }
                    }
                }
                out.push(ChatMessageView {
                    id: next_id,
                    role,
                    blocks,
                    status: MessageStatus::Complete,
                });
                next_id += 1;
            }
            SidecarEvent::Complete { .. } | SidecarEvent::Error { .. } => break,
            _ => {}
        }
    }
    let _ = spawned.follow_up_tx.send(FollowUp::Close);
    Ok(out)
}
