//! Serde types for the JSONL events emitted by `agent/runner.mjs`.
//!
//! Event taxonomy comes from `src_react_legacy/lib/types.ts::ToolEvent` and the
//! `emit(...)` calls in `agent/runner.mjs`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SidecarEvent {
    /// Streaming text chunk.
    TextDelta {
        delta: String,
        #[serde(default)]
        timestamp: Option<i64>,
    },
    /// Tool call beginning.
    ToolStart {
        tool: String,
        #[serde(default)]
        args: Option<BTreeMap<String, serde_json::Value>>,
        #[serde(default)]
        timestamp: Option<i64>,
    },
    /// Tool call ending.
    ToolEnd {
        tool: String,
        #[serde(default)]
        args: Option<BTreeMap<String, serde_json::Value>>,
        #[serde(default)]
        result: Option<String>,
        #[serde(default)]
        timestamp: Option<i64>,
    },
    /// Error from the sidecar.
    Error {
        #[serde(default)]
        error: Option<String>,
        #[serde(default)]
        timestamp: Option<i64>,
    },
    /// Sidecar is about to call `query()` for a new turn.
    TurnStart {
        #[serde(default)]
        timestamp: Option<i64>,
    },
    /// Turn completed — carries result metadata from `SDKResultMessage`.
    Complete {
        #[serde(rename = "durationMs", default)]
        duration_ms: Option<u64>,
        #[serde(rename = "numTurns", default)]
        num_turns: Option<u32>,
        #[serde(rename = "totalCostUsd", default)]
        total_cost_usd: Option<f64>,
        #[serde(default)]
        timestamp: Option<i64>,
    },
    /// Agent is ready for the next follow-up.
    Ready {
        #[serde(default)]
        timestamp: Option<i64>,
    },
    /// SDK session ID — emitted once per session after the first API response.
    SessionId {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(default)]
        timestamp: Option<i64>,
    },
    /// `list` mode: one entry per recent session.
    Session {
        #[serde(rename = "sessionId")]
        session_id: String,
        summary: String,
        #[serde(rename = "lastModified")]
        last_modified: i64,
        #[serde(rename = "firstPrompt", default)]
        first_prompt: Option<String>,
    },
    /// `messages` mode: one entry per historical chat message.
    ChatMessage {
        role: String,
        #[serde(default)]
        content: Option<String>,
        #[serde(rename = "toolEvents", default)]
        tool_events: Option<Vec<SidecarEvent>>,
        #[serde(default)]
        timestamp: Option<i64>,
        #[serde(rename = "durationMs", default)]
        duration_ms: Option<u64>,
    },
}
