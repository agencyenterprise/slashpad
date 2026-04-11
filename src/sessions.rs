//! Session history — wraps one-shot sidecar runs in `list` and `messages`
//! modes to fetch recent sessions and past-session messages.

use crate::sidecar::{self, FollowUp, Payload, SidecarEvent};
use crate::state::{ChatMessageView, ContentBlock, MessageStatus, Role, SessionInfo};

/// Spawn `runner.mjs list` and collect recent sessions.
pub async fn list_recent() -> anyhow::Result<Vec<SessionInfo>> {
    let home = sidecar::launchpad_home()?;
    let payload = Payload::list(home.to_string_lossy().to_string());
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

/// Spawn `runner.mjs messages <sessionId>` and collect historical chat messages.
pub async fn load_messages(session_id: &str) -> anyhow::Result<Vec<ChatMessageView>> {
    let home = sidecar::launchpad_home()?;
    let payload = Payload::messages(session_id.to_string(), home.to_string_lossy().to_string());
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
