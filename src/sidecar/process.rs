//! Spawns `node agent/runner.mjs <base64_payload>` and plumbs stdin/stdout.
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
    // Default: launchpad's own project dir under $HOME/dev/launchpad. In production
    // installs this would come from a bundled resource; for the dev path we can
    // resolve it relative to the current working directory.
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    cwd.join("agent").join("runner.mjs")
}

/// Spawn the sidecar for the given payload and return handles for event draining
/// and follow-up message sending.
pub fn spawn(payload: Payload) -> std::io::Result<SpawnedSidecar> {
    let encoded = payload.to_base64_arg();
    let runner = runner_path();

    let mut cmd = Command::new("node");
    cmd.arg(&runner)
        .arg(&encoded)
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
