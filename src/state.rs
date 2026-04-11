//! Application state types.
//!
//! Mirrors `src_react_legacy/lib/types.ts` + the state machine from `usePalette.ts`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
