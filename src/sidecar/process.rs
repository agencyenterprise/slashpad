//! Spawns `bun agent/runner.mjs <base64_payload>` (or node as fallback) and plumbs stdin/stdout.
//!
//! - stdout: newline-delimited JSON → parsed into `SidecarEvent`s → sent on a
//!   broadcast-like mpsc::UnboundedSender<SidecarEvent>. When iced subscribes,
//!   it drains this channel as a Stream.
//! - stdin: JSON lines `{"type":"message","content":"..."}` or `{"type":"close"}`
//!   written by the writer task.

use std::path::PathBuf;
use std::process::Stdio;

use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;

use super::events::SidecarEvent;
use super::payload::Payload;

pub struct SpawnedSidecar {
    /// Child process handle — dropped ⇒ kill on drop via tokio's default kill_on_drop.
    pub child: Child,
    /// Sender for follow-up messages; consumed by the writer task.
    pub follow_up_tx: mpsc::UnboundedSender<FollowUp>,
    /// Receiver that streams out events the UI should consume.
    pub event_rx: mpsc::UnboundedReceiver<SidecarEvent>,
}

#[derive(Debug, Clone)]
pub enum FollowUp {
    Message(String),
    Close,
}

pub fn runner_path() -> PathBuf {
    // 1. Explicit override via environment variable.
    if let Ok(path) = std::env::var("SLASHPAD_RUNNER") {
        return PathBuf::from(path);
    }

    // 2. Relative to the binary — .app bundle layout.
    if let Ok(exe) = std::env::current_exe() {
        if let Ok(canonical) = exe.canonicalize() {
            if let Some(exe_dir) = canonical.parent() {
                // .app bundle: Contents/MacOS/slashpad → Contents/Resources/agent/runner.mjs
                let bundle_path = exe_dir.join("../Resources/agent/runner.mjs");
                if bundle_path.exists() {
                    return bundle_path;
                }
            }
        }
    }

    // 3. CWD-relative fallback for development (cargo run from repo root).
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    cwd.join("agent").join("runner.mjs")
}

/// Resolve the JS runtime (bun or node) used to execute runner.mjs.
///
/// Priority:
/// 1. `SLASHPAD_RUNTIME` env var — explicit override.
/// 2. Bundled `bun` inside the `.app`.
/// 3. `bun` on PATH — developer machines with bun installed.
/// 4. `node` on PATH — legacy fallback.
fn runtime_path() -> PathBuf {
    if let Ok(p) = std::env::var("SLASHPAD_RUNTIME") {
        return PathBuf::from(p);
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Ok(canonical) = exe.canonicalize() {
            if let Some(exe_dir) = canonical.parent() {
                // .app bundle: Contents/MacOS/slashpad → Contents/Resources/bin/bun
                let bundle_bun = exe_dir.join("../Resources/bin/bun");
                if bundle_bun.exists() {
                    return bundle_bun;
                }
            }
        }
    }

    if command_exists("bun") {
        return PathBuf::from("bun");
    }

    PathBuf::from("node")
}

fn command_exists(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Build a PATH that includes common binary locations that may be missing
/// when launched as a macOS service (launchd provides only /usr/bin:/bin:/usr/sbin:/sbin).
fn augmented_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let extra = [
        format!("{home}/.local/bin"),        // Claude CLI (official installer)
        "/opt/homebrew/bin".to_string(),      // Homebrew (Apple Silicon)
        "/usr/local/bin".to_string(),         // Homebrew (Intel) / common tools
    ];
    let current = std::env::var("PATH").unwrap_or_default();
    let mut parts: Vec<&str> = extra.iter().map(|s| s.as_str()).collect();
    parts.extend(current.split(':'));
    parts.dedup();
    parts.join(":")
}

/// Spawn the sidecar for the given payload and return handles for event draining
/// and follow-up message sending.
pub fn spawn(payload: Payload) -> std::io::Result<SpawnedSidecar> {
    let encoded = payload.to_base64_arg();
    let runner = runner_path();
    let runtime = runtime_path();

    let mut cmd = Command::new(&runtime);
    cmd.arg(&runner)
        .arg(&encoded)
        .env("PATH", augmented_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn()?;

    let stdout = child.stdout.take().expect("stdout requested");
    let stderr = child.stderr.take().expect("stderr requested");
    let stdin = child.stdin.take().expect("stdin requested");

    let (event_tx, event_rx) = mpsc::unbounded_channel::<SidecarEvent>();
    let (follow_up_tx, follow_up_rx) = mpsc::unbounded_channel::<FollowUp>();

    // Reader task: parse JSONL → events
    {
        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<SidecarEvent>(&line) {
                    Ok(event) => {
                        if event_tx.send(event).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("[sidecar] unparseable line: {line} ({e})");
                    }
                }
            }
        });
    }

    // Stderr drain: capture stderr and, on nonzero exit, surface it as an error event.
    {
        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            let mut stderr_reader = BufReader::new(stderr);
            let mut buf = Vec::new();
            let _ = tokio::io::copy(&mut stderr_reader, &mut buf).await;
            let text = String::from_utf8_lossy(&buf).trim().to_string();
            if !text.is_empty() {
                let _ = event_tx.send(SidecarEvent::Error {
                    error: Some(text),
                    timestamp: None,
                });
            }
        });
    }

    // Writer task: pump follow-ups into stdin
    tokio::spawn(writer_task(stdin, follow_up_rx));

    Ok(SpawnedSidecar {
        child,
        follow_up_tx,
        event_rx,
    })
}

async fn writer_task(mut stdin: ChildStdin, mut rx: mpsc::UnboundedReceiver<FollowUp>) {
    while let Some(msg) = rx.recv().await {
        let line = match msg {
            FollowUp::Message(content) => {
                let obj = json!({ "type": "message", "content": content });
                format!("{obj}\n")
            }
            FollowUp::Close => {
                let obj = json!({ "type": "close" });
                format!("{obj}\n")
            }
        };
        if stdin.write_all(line.as_bytes()).await.is_err() {
            break;
        }
        let _ = stdin.flush().await;
    }
}
