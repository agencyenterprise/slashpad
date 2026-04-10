//! Base64-encoded JSON payload passed to `runner.mjs` as argv[2].
//!
//! Matches the schema in `src_react_legacy/lib/agent.ts`.

use base64::prelude::*;
use serde::Serialize;

pub const SYSTEM_PROMPT: &str = r#"You are Launchpad, a fast personal AI assistant running as a desktop command palette.
You help the user by executing tasks quickly and concisely.

Guidelines:
- Be extremely concise. This is a command palette, not a chat.
- When using tools, do so without asking for confirmation unless destructive.
- Format output as clean markdown.
- Prioritize speed and directness over politeness.

## Composio — External App Integrations

You have access to 1000+ app integrations through the Composio CLI.
Bias toward action: run `composio search <task>`, then `composio execute <slug>`.
Input validation, auth checks, and error messages are built in — just try it.

### Installation
If `composio` is not found or errors on startup, install it:
  curl -fsSL https://composio.dev/install | bash
Then authenticate: `composio login`

### Core Commands

**search** — Find tools. Use this first — describe what you need in natural language.
  composio search <query> [--toolkits text]

**execute** — Run a tool. Handles input validation and auth checks automatically.
  If auth is missing, the error tells you what to run. Use aggressively.
  composio execute <slug> [-d, --data text] [--dry-run] [--get-schema]

**link** — Connect an account. Only needed when execute tells you to — don't preemptively link.
  composio link <toolkit> [--no-wait]

**run** — Run inline TS/JS code with shimmed CLI commands; injected execute(), search(), proxy(), experimental_subAgent(), and z (zod).
  composio run <code> [-- ...args] | run [-f, --file text] [-- ...args] [--dry-run]

**proxy** — curl-like access to any toolkit API through Composio using the linked account.
  composio proxy <url> --toolkit text [-X method] [-H header]... [-d data]

**tools** — Inspect known tools.
  composio tools info <slug>
  composio tools list <toolkit>

**artifacts** — Inspect the cwd-scoped session artifact directory and history.
  composio artifacts cwd

### Workflow
search → execute. If execute fails with an auth error, run link, then retry.
"#;

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
    #[serde(rename = "systemPrompt")]
    pub system_prompt: String,
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
            system_prompt: SYSTEM_PROMPT.to_string(),
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
