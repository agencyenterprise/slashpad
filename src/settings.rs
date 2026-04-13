//! Persistent settings stored at `~/.launchpad/settings.json`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const DEFAULT_HOTKEY: &str = "Ctrl+Space";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    #[serde(default, rename = "apiKey")]
    pub api_key: Option<String>,
    /// When true, the sidecar runs without a forwarded API key and
    /// falls back to the user's `claude login` session. When false,
    /// `api_key` is forwarded to the Agent SDK. Defaults to true so a
    /// fresh install uses the subscription out of the box.
    #[serde(default = "default_true", rename = "useSubscription")]
    pub use_subscription: bool,
}

fn default_hotkey() -> String {
    DEFAULT_HOTKEY.to_string()
}

fn default_true() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            hotkey: DEFAULT_HOTKEY.to_string(),
            api_key: None,
            use_subscription: true,
        }
    }
}

fn settings_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(home).join(".launchpad");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("settings.json")
}

impl AppSettings {
    pub fn load_or_default() -> Self {
        match std::fs::read_to_string(settings_path()) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string());
        std::fs::write(settings_path(), json)
    }
}
