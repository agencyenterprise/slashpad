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
    /// Cmd+P picker for switching the Claude project directory. The
    /// unfiltered list (sourced from `~/.claude/projects/`) lives on
    /// `Slashpad`; the live query is `self.input`.
    ProjectPicker,
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
        /// SDK tool_use id; used to match a later `ToolEnd`/`ToolResult`.
        /// Optional for backwards compatibility with older sidecar events.
        tool_use_id: Option<String>,
        args: BTreeMap<String, serde_json::Value>,
    },
    ToolEnd {
        tool: String,
        tool_use_id: Option<String>,
        args: BTreeMap<String, serde_json::Value>,
        result: Option<String>,
        /// Set by a `ToolResult` event with `is_error: true`. The UI renders
        /// errored tool calls with a red ✗ instead of the green ✓.
        is_error: bool,
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
    /// Whether the tool-call section is expanded (user toggled).
    pub tools_expanded: bool,
    /// SDK-reported turn duration in milliseconds (from `SDKResultMessage`).
    pub result_duration_ms: Option<u64>,
}

impl ChatMessageView {
    pub fn user(id: u64, text: String) -> Self {
        Self {
            id,
            role: Role::User,
            blocks: vec![ContentBlock::text(text)],
            status: MessageStatus::Complete,
            tools_expanded: false,
            result_duration_ms: None,
        }
    }

    pub fn assistant_streaming(id: u64) -> Self {
        Self {
            id,
            role: Role::Assistant,
            blocks: Vec::new(),
            status: MessageStatus::Streaming,
            tools_expanded: false,
            result_duration_ms: None,
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

/// Tag value used to mark a session as archived. Archived sessions are
/// filtered out of the idle list by `past_session_rows`. Persistence is
/// handled by the Claude Agent SDK's `tagSession` — no local registry.
pub const TAG_ARCHIVED: &str = "archived";

/// Tag prefix for pinned sessions. Stored as `"pinned:<unix_millis>"`
/// so ordering within the pinned block reflects pin time — most
/// recently pinned sorts to the bottom. The bare `"pinned"` form is
/// also recognized as a legacy pin with unknown timestamp.
pub const TAG_PINNED_PREFIX: &str = "pinned";

/// True when a tag marks a session as archived.
pub fn is_archived(tag: Option<&str>) -> bool {
    tag == Some(TAG_ARCHIVED)
}

/// True when a tag marks a session as pinned (either the new
/// `pinned:<ts>` form or the legacy bare `pinned`).
pub fn is_pinned(tag: Option<&str>) -> bool {
    match tag {
        Some(t) => t == TAG_PINNED_PREFIX || t.starts_with("pinned:"),
        None => false,
    }
}

/// Extract the unix-millis pin timestamp from a `pinned:<ts>` tag.
/// Returns `None` for legacy bare `pinned` tags — callers sort those
/// as if pinned at epoch 0 (i.e. above more-recently-pinned items).
pub fn pin_timestamp(tag: Option<&str>) -> Option<i64> {
    tag?.strip_prefix("pinned:")?.parse::<i64>().ok()
}

/// Build a fresh pin tag with the current unix-millis timestamp so
/// newly-pinned sessions sort to the bottom of the pinned group.
pub fn new_pin_tag() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    pin_tag_with(millis)
}

/// Build a pin tag with a specific unix-millis timestamp. Used when
/// reordering within the pinned block: two neighbors swap their
/// timestamps so one slides above/below the other.
pub fn pin_tag_with(timestamp_ms: i64) -> String {
    format!("{TAG_PINNED_PREFIX}:{timestamp_ms}")
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
    /// SDK-backed tag (see `tagSession`). `None` for untagged sessions.
    #[serde(default)]
    pub tag: Option<String>,
}

// ---------- multi-chat state ----------

/// Identifier for a chat *within this Slashpad process*. Unrelated to
/// Claude's `session_id` (which is assigned by the SDK and doesn't exist
/// until after the first turn's `result` event). We need an identity
/// from the moment we spawn the sidecar, so new chats get a local
/// monotonically increasing `ChatId` allocated by `Slashpad`, and the
/// Claude `session_id` is stored as a separate field on `ChatState`
/// once the `SessionId` event arrives.
pub type ChatId = u64;

/// Fingerprint of a display, used as the key for per-screen palette
/// drag memory. Built from an `NSScreen` frame rect: stable within a
/// session as long as the display configuration doesn't change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScreenKey {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

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
    /// Set when the sidecar emits `TurnStart` at the beginning of each
    /// turn. The chat-level "Working..." indicator reads elapsed time
    /// from this. Reset on each new turn.
    pub turn_submitted_at: Option<std::time::Instant>,
    /// Wall-clock time of the last sidecar event, as unix millis. Bumped
    /// by every `apply_event` call so the idle list can render a
    /// "last activity" relative timestamp for chats that aren't mid-turn.
    pub last_activity_ms: i64,
}

/// Current wall-clock time as unix millis. Used for `last_activity_ms`
/// bookkeeping; same calculation as `idle_list::format_relative`'s `now`.
fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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
            turn_submitted_at: None,
            last_activity_ms: now_ms(),
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
            turn_submitted_at: None,
            last_activity_ms: now_ms(),
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

    /// Mark an in-flight turn as cancelled by the user. Seals the
    /// current streaming assistant bubble (if any) as Complete so the
    /// partial content renders normally, and flips the chat back to
    /// `Idle` so the next follow-up is allowed. Callers are responsible
    /// for dropping the `SpawnedSidecar` that was producing the turn.
    pub fn mark_cancelled(&mut self) {
        if let Some(msg) = self.current_assistant_mut() {
            msg.status = MessageStatus::Complete;
        }
        self.current_assistant_id = None;
        self.status = ChatStatus::Idle;
        self.turn_submitted_at = None;
        self.last_activity_ms = now_ms();
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
    /// the old `Slashpad::process_sidecar_event` (app.rs pre-refactor)
    /// but scoped to a single chat, and drives `status` transitions.
    pub fn apply_event(&mut self, event: SidecarEvent) {
        self.last_activity_ms = now_ms();
        match event {
            SidecarEvent::TurnStart { .. } => {
                self.turn_submitted_at = Some(std::time::Instant::now());
                self.promote_to_streaming();
                self.ensure_streaming_assistant();
            }
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
            SidecarEvent::ToolStart {
                tool,
                tool_use_id,
                args,
                ..
            } => {
                self.promote_to_streaming();
                self.ensure_streaming_assistant();
                let args = args.unwrap_or_default();
                if let Some(msg) = self.current_assistant_mut() {
                    msg.blocks.push(ContentBlock::ToolStart {
                        tool,
                        tool_use_id,
                        args,
                    });
                }
            }
            SidecarEvent::ToolEnd {
                tool,
                tool_use_id,
                args,
                result,
                ..
            } => {
                self.promote_to_streaming();
                self.ensure_streaming_assistant();
                let args = args.unwrap_or_default();
                if let Some(msg) = self.current_assistant_mut() {
                    // Prefer matching by tool_use_id (unambiguous across
                    // multiple concurrent tool calls of the same name);
                    // fall back to tool-name match for older sidecar events
                    // that don't carry an id.
                    let replaced = msg.blocks.iter_mut().rev().find(|b| match b {
                        ContentBlock::ToolStart {
                            tool_use_id: id,
                            tool: t,
                            ..
                        } => match (id, &tool_use_id) {
                            (Some(a), Some(b)) => a == b,
                            _ => t == &tool,
                        },
                        _ => false,
                    });
                    if let Some(slot) = replaced {
                        *slot = ContentBlock::ToolEnd {
                            tool,
                            tool_use_id,
                            args,
                            result,
                            is_error: false,
                        };
                    } else {
                        msg.blocks.push(ContentBlock::ToolEnd {
                            tool,
                            tool_use_id,
                            args,
                            result,
                            is_error: false,
                        });
                    }
                }
            }
            SidecarEvent::ToolResult {
                tool_use_id,
                content,
                is_error,
                ..
            } => {
                // Find the ToolEnd this result belongs to and patch in the
                // real result text + error flag. Walk messages newest-first
                // — the match is almost always in the current assistant
                // bubble, but in rare multi-turn patterns a late-arriving
                // tool_result can land after Complete.
                let patched = self
                    .messages
                    .iter_mut()
                    .rev()
                    .flat_map(|m| m.blocks.iter_mut())
                    .find_map(|b| match b {
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
            SidecarEvent::Complete { duration_ms, .. } => {
                if let Some(msg) = self.current_assistant_mut() {
                    msg.status = MessageStatus::Complete;
                    msg.result_duration_ms = duration_ms;
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
