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

/// A content block inside an assistant message — matches the TS
/// `ContentBlock` shape: either plain text or a tool event.
#[derive(Debug, Clone)]
pub enum ContentBlock {
    Text(String),
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
            blocks: vec![ContentBlock::Text(text)],
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
            if let ContentBlock::Text(t) = block {
                out.push_str(t);
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
