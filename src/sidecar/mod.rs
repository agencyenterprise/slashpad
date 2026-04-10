//! Sidecar: spawns `node agent/runner.mjs` and manages JSONL IPC.

pub mod events;
pub mod payload;
pub mod process;

pub use events::SidecarEvent;
pub use payload::Payload;
pub use process::{spawn, FollowUp, SpawnedSidecar};

use std::path::PathBuf;

/// Determine the launchpad home directory (`~/.launchpad`), creating it if
/// missing. This is the cwd passed to `runner.mjs`.
pub fn launchpad_home() -> std::io::Result<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(home).join(".launchpad");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
