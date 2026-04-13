//! Base64-encoded JSON payload passed to `runner.mjs` as argv[2].
//!
//! Matches the schema in `src_react_legacy/lib/agent.ts`.

use base64::prelude::*;
use serde::Serialize;

/// Default system prompt bundled into the binary. Seeded to
/// `~/.launchpad/CLAUDE.md` on first run (see `sidecar::seed_default_claude_md`).
/// The Claude Agent SDK auto-loads this file via `settingSources: ["project"]`
/// in `runner.mjs` — we don't pass it through the payload.
pub const DEFAULT_CLAUDE_MD: &str = include_str!("../../bundled-prompts/CLAUDE.md");

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Payload {
    Chat(ChatPayload),
    List(ListPayload),
    Messages(MessagesPayload),
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatPayload {
    pub mode: &'static str, // "chat"
    pub prompt: String,
    #[serde(rename = "apiKey", skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListPayload {
    pub mode: &'static str, // "list"
    pub cwd: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessagesPayload {
    pub mode: &'static str, // "messages"
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub cwd: String,
}

impl Payload {
    pub fn chat(
        prompt: String,
        cwd: String,
        api_key: Option<String>,
        resume: Option<String>,
    ) -> Self {
        Payload::Chat(ChatPayload {
            mode: "chat",
            prompt,
            api_key,
            cwd,
            resume,
        })
    }

    pub fn list(cwd: String) -> Self {
        Payload::List(ListPayload { mode: "list", cwd })
    }

    pub fn messages(session_id: String, cwd: String) -> Self {
        Payload::Messages(MessagesPayload {
            mode: "messages",
            session_id,
            cwd,
        })
    }

    pub fn to_base64_arg(&self) -> String {
        let json = serde_json::to_string(self).expect("payload should always serialize");
        BASE64_STANDARD.encode(json.as_bytes())
    }
}
