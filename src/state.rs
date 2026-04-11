//! Application state types.
//!
//! Mirrors `src_react_legacy/lib/types.ts` + the state machine from `usePalette.ts`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::sidecar::SidecarEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Idle,
    Skills,
    Chatting,
    Settings,
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageStatus {
    Streaming,
    Complete,
    Error,
}

/// A content block inside a chat message — matches the TS
/// `ContentBlock` shape: either text (user prompt or assistant reply)
/// or a tool event. Text blocks carry both the raw string and a
/// pre-parsed list of `iced::widget::markdown::Item`s; the parsed
/// form is what the markdown widget renders, and the raw form is
/// what we append new deltas to (and what `flat_text` returns). We
/// keep them in sync inside `ContentBlock::text()` / the streaming
/// append path in `app.rs`.
#[derive(Debug, Clone)]
pub enum ContentBlock {
    Text {
        raw: String,
        parsed: Vec<iced::widget::markdown::Item>,
    },
    ToolStart {
        tool: String,
        args: BTreeMap<String, serde_json::Value>,
    },
    ToolEnd {
        tool: String,
        args: BTreeMap<String, serde_json::Value>,
        result: Option<String>,
    },
    Error(String),
}

impl ContentBlock {
    /// Build a `Text` block from a raw string, parsing markdown once.
    pub fn text(raw: String) -> Self {
        let parsed = iced::widget::markdown::parse(&raw).collect();
        ContentBlock::Text { raw, parsed }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub struct ChatMessageView {
    pub id: u64,
    pub role: Role,
    pub blocks: Vec<ContentBlock>,
    pub status: MessageStatus,
}

impl ChatMessageView {
    pub fn user(id: u64, text: String) -> Self {
        Self {
            id,
            role: Role::User,
            blocks: vec![ContentBlock::text(text)],
            status: MessageStatus::Complete,
        }
    }

    pub fn assistant_streaming(id: u64) -> Self {
        Self {
            id,
            role: Role::Assistant,
            blocks: Vec::new(),
            status: MessageStatus::Streaming,
        }
    }

    /// Full flat text of all text blocks in this message, used for copy-result.
    pub fn flat_text(&self) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            if let ContentBlock::Text { raw, .. } = block {
                out.push_str(raw);
            }
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub summary: String,
    #[serde(rename = "lastModified")]
    pub last_modified: i64,
    #[serde(rename = "firstPrompt", default)]
    pub first_prompt: Option<String>,
}

// ---------- multi-chat state ----------

/// Identifier for a chat *within this Launchpad process*. Unrelated to
/// Claude's `session_id` (which is assigned by the SDK and doesn't exist
/// until after the first turn's `result` event). We need an identity
/// from the moment we spawn the sidecar, so new chats get a local
/// monotonically increasing `ChatId` allocated by `Launchpad`, and the
/// Claude `session_id` is stored as a separate field on `ChatState`
/// once the `SessionId` event arrives.
pub type ChatId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatStatus {
    /// Sidecar spawned, no events yet.
    Initializing,
    /// Streaming deltas / tool calls.
    Streaming,
    /// Agent ready for the next follow-up (post-`Ready`).
    Idle,
    /// An `Error` event was received.
    Error,
    /// Sidecar process exited (stdout closed).
    Closed,
}

#[derive(Debug)]
pub struct ChatState {
    pub id: ChatId,
    /// Truncated first user prompt, shown as the row title in the idle list.
    pub title: String,
    pub messages: Vec<ChatMessageView>,
    /// Populated when the sidecar emits `SessionId`. Used for `resume`
    /// payloads when respawning, and for deduping against past-session
    /// rows in the idle list.
    pub session_id: Option<String>,
    pub current_assistant_id: Option<u64>,
    pub next_msg_id: u64,
    pub status: ChatStatus,
    pub started_at: std::time::Instant,
}

impl ChatState {
    /// Build a fresh chat state around a first user prompt. The user
    /// bubble is pre-inserted as message 1; status starts as
    /// `Initializing` until the first event arrives.
    pub fn new(id: ChatId, first_prompt: &str) -> Self {
        let title = Self::derive_title(first_prompt);
        let mut me = Self {
            id,
            title,
            messages: Vec::new(),
            session_id: None,
            current_assistant_id: None,
            next_msg_id: 1,
            status: ChatStatus::Initializing,
            started_at: std::time::Instant::now(),
        };
        let user_id = me.alloc_msg_id();
        me.messages
            .push(ChatMessageView::user(user_id, first_prompt.to_string()));
        me
    }

    /// Build a chat state for a session being resumed from disk. No
    /// user bubble is inserted — history messages are loaded later via
    /// the one-shot `messages`-mode sidecar.
    pub fn resumed(id: ChatId, session_id: String, title: String) -> Self {
        Self {
            id,
            title,
            messages: Vec::new(),
            session_id: Some(session_id),
            current_assistant_id: None,
            next_msg_id: 1,
            // Treat resumed-from-disk as ready for follow-up — there's
            // no in-flight turn, we're just viewing history.
            status: ChatStatus::Idle,
            started_at: std::time::Instant::now(),
        }
    }

    pub fn alloc_msg_id(&mut self) -> u64 {
        let id = self.next_msg_id;
        self.next_msg_id += 1;
        id
    }

    /// Ensure the last message is a streaming assistant bubble we can
    /// append blocks to. If not, push a new one and record its id.
    pub fn ensure_streaming_assistant(&mut self) {
        let needs_new = match self.messages.last() {
            Some(m) => m.role != Role::Assistant || m.status != MessageStatus::Streaming,
            None => true,
        };
        if needs_new {
            let id = self.alloc_msg_id();
            self.current_assistant_id = Some(id);
            self.messages.push(ChatMessageView::assistant_streaming(id));
        }
    }

    pub fn current_assistant_mut(&mut self) -> Option<&mut ChatMessageView> {
        let id = self.current_assistant_id?;
        self.messages.iter_mut().find(|m| m.id == id)
    }

    pub fn push_error(&mut self, error: String) {
        let id = self.alloc_msg_id();
        self.current_assistant_id = Some(id);
        let mut msg = ChatMessageView::assistant_streaming(id);
        msg.status = MessageStatus::Error;
        msg.blocks.push(ContentBlock::Error(error));
        self.messages.push(msg);
    }

    /// Apply a `SidecarEvent` to this chat's state. Mirrors the body of
    /// the old `Launchpad::process_sidecar_event` (app.rs pre-refactor)
    /// but scoped to a single chat, and drives `status` transitions.
    pub fn apply_event(&mut self, event: SidecarEvent) {
        match event {
            SidecarEvent::Ready { .. } => {
                self.status = ChatStatus::Idle;
            }
            SidecarEvent::SessionId { session_id, .. } => {
                self.session_id = Some(session_id);
            }
            SidecarEvent::TextDelta { delta, .. } => {
                self.promote_to_streaming();
                self.ensure_streaming_assistant();
                if let Some(msg) = self.current_assistant_mut() {
                    match msg.blocks.last_mut() {
                        Some(ContentBlock::Text { raw, parsed }) => {
                            raw.push_str(&delta);
                            *parsed = iced::widget::markdown::parse(raw).collect();
                        }
                        _ => msg.blocks.push(ContentBlock::text(delta)),
                    }
                }
            }
            SidecarEvent::ToolStart { tool, args, .. } => {
                self.promote_to_streaming();
                self.ensure_streaming_assistant();
                let args = args.unwrap_or_default();
                if let Some(msg) = self.current_assistant_mut() {
                    msg.blocks.push(ContentBlock::ToolStart { tool, args });
                }
            }
            SidecarEvent::ToolEnd {
                tool, args, result, ..
            } => {
                self.promote_to_streaming();
                self.ensure_streaming_assistant();
                let args = args.unwrap_or_default();
                if let Some(msg) = self.current_assistant_mut() {
                    msg.blocks
                        .push(ContentBlock::ToolEnd { tool, args, result });
                }
            }
            SidecarEvent::Error { error, .. } => {
                let err = error.unwrap_or_else(|| "An error occurred".to_string());
                self.status = ChatStatus::Error;
                if let Some(msg) = self.current_assistant_mut() {
                    msg.status = MessageStatus::Error;
                    msg.blocks.push(ContentBlock::Error(err));
                } else {
                    self.push_error(err);
                }
            }
            SidecarEvent::Complete { .. } => {
                if let Some(msg) = self.current_assistant_mut() {
                    msg.status = MessageStatus::Complete;
                }
                self.current_assistant_id = None;
                // `Complete` marks the assistant bubble done; `Ready`
                // (which follows) is what flips the chat to Idle.
            }
            SidecarEvent::Session { .. } | SidecarEvent::ChatMessage { .. } => {
                // These arrive from list/messages modes and are consumed
                // by sessions.rs directly, not through this stream.
            }
        }
    }

    fn promote_to_streaming(&mut self) {
        if matches!(self.status, ChatStatus::Initializing | ChatStatus::Idle) {
            self.status = ChatStatus::Streaming;
        }
    }

    fn derive_title(prompt: &str) -> String {
        let trimmed = prompt.trim();
        if trimmed.chars().count() <= 60 {
            trimmed.to_string()
        } else {
            let mut s: String = trimmed.chars().take(57).collect();
            s.push_str("...");
            s
        }
    }
}
