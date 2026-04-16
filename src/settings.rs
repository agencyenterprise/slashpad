//! Persistent settings stored at `~/.slashpad/settings.json`.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const DEFAULT_HOTKEY: &str = "Ctrl+Space";

/// Which terminal emulator Slashpad should use when opening a chat
/// session in the Claude Code CLI. Rendered in the settings dropdown
/// and consumed by `terminal::open_claude_resume`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PreferredTerminal {
    #[default]
    Terminal,
    #[serde(rename = "iterm")]
    ITerm,
    Ghostty,
    Warp,
    Alacritty,
    Kitty,
    #[serde(rename = "wezterm")]
    WezTerm,
}

impl PreferredTerminal {
    pub const ALL: [PreferredTerminal; 7] = [
        PreferredTerminal::Terminal,
        PreferredTerminal::ITerm,
        PreferredTerminal::Ghostty,
        PreferredTerminal::Warp,
        PreferredTerminal::Alacritty,
        PreferredTerminal::Kitty,
        PreferredTerminal::WezTerm,
    ];

    /// Best-effort auto-detection of the user's preferred terminal.
    /// Called exactly once, the first time `settings.json` is loaded
    /// without an explicit `preferredTerminal` key — after that the
    /// saved value wins, so a user's pick from the Settings dropdown
    /// is never overridden.
    ///
    /// Signal priority:
    ///
    /// 1. `$TERM_PROGRAM` — set by every mainstream terminal. Strong
    ///    when present (Slashpad was clearly launched from a shell
    ///    in that terminal); usually unset when Slashpad is started
    ///    via hotkey/tray because `launchd` hands down an empty env.
    /// 2. `/Applications/<name>.app` on disk — if the user went to
    ///    the trouble of installing a non-default terminal, that's
    ///    the one they want. Within the set of installed terminals
    ///    we prefer the more specialized / opt-in ones (Ghostty,
    ///    iTerm, WezTerm, Kitty, Alacritty, Warp) over the built-in
    ///    Terminal.app. Terminal.app is always installed so it's
    ///    the unconditional fallback.
    pub fn detect() -> Self {
        if let Ok(term_program) = std::env::var("TERM_PROGRAM") {
            // Values observed from each terminal's shell
            // integration; kept loose so future capitalization
            // changes don't break detection.
            let t = term_program.to_ascii_lowercase();
            if t == "apple_terminal" {
                return PreferredTerminal::Terminal;
            }
            if t.contains("iterm") {
                return PreferredTerminal::ITerm;
            }
            if t.contains("ghostty") {
                return PreferredTerminal::Ghostty;
            }
            if t.contains("warp") {
                return PreferredTerminal::Warp;
            }
            if t.contains("wezterm") {
                return PreferredTerminal::WezTerm;
            }
            if t.contains("alacritty") {
                return PreferredTerminal::Alacritty;
            }
            if t.contains("kitty") {
                return PreferredTerminal::Kitty;
            }
            // Unknown TERM_PROGRAM (e.g. `vscode`, `tmux`) — fall
            // through to the /Applications probe.
        }

        let candidates = [
            ("/Applications/Ghostty.app", PreferredTerminal::Ghostty),
            ("/Applications/iTerm.app", PreferredTerminal::ITerm),
            ("/Applications/WezTerm.app", PreferredTerminal::WezTerm),
            ("/Applications/kitty.app", PreferredTerminal::Kitty),
            ("/Applications/Alacritty.app", PreferredTerminal::Alacritty),
            ("/Applications/Warp.app", PreferredTerminal::Warp),
        ];
        for (path, term) in candidates {
            if std::path::Path::new(path).exists() {
                return term;
            }
        }
        PreferredTerminal::Terminal
    }
}

impl fmt::Display for PreferredTerminal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            PreferredTerminal::Terminal => "Terminal",
            PreferredTerminal::ITerm => "iTerm",
            PreferredTerminal::Ghostty => "Ghostty",
            PreferredTerminal::Warp => "Warp",
            PreferredTerminal::Alacritty => "Alacritty",
            PreferredTerminal::Kitty => "Kitty",
            PreferredTerminal::WezTerm => "WezTerm",
        };
        f.write_str(label)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    /// When true, the sidecar runs without a forwarded API key and
    /// falls back to the user's `claude login` session. When false,
    /// the API key stored in the OS keychain (see `secrets` module)
    /// is forwarded to the Agent SDK. Defaults to true so a fresh
    /// install uses the subscription out of the box.
    #[serde(default = "default_true", rename = "useSubscription")]
    pub use_subscription: bool,
    /// Terminal emulator used when the user presses Cmd+T from a chat
    /// to resume the session in the Claude Code CLI. Defaults to the
    /// built-in Terminal.app so it works on a fresh macOS install.
    #[serde(default, rename = "preferredTerminal")]
    pub preferred_terminal: PreferredTerminal,
    /// When true, the sidecar loads user-level settings, skills, and
    /// hooks from `~/.claude/` (via the Agent SDK's `settingSources:
    /// ["user", "project"]`) and the palette's skill list is augmented
    /// with skills from `~/.claude/skills/`. Off by default so a fresh
    /// install stays isolated to Slashpad's own `~/.slashpad/` scope.
    #[serde(default, rename = "loadUserSettings")]
    pub load_user_settings: bool,
    /// Directory Claude Code runs in (`cwd` passed to the sidecar).
    /// Chosen via the Cmd+P project picker; persists across restarts
    /// so the user stays in their last-selected project. `None` means
    /// fall back to `~/.slashpad` (the default). A saved path that
    /// no longer exists on disk is ignored at load time.
    #[serde(default, rename = "selectedProjectPath")]
    pub selected_project_path: Option<String>,
    /// When true, Slashpad launches automatically at login. Only
    /// applicable to `.app` installs (Homebrew uses `brew services`).
    #[serde(default, rename = "launchAtLogin")]
    pub launch_at_login: bool,
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
            use_subscription: true,
            preferred_terminal: PreferredTerminal::default(),
            load_user_settings: false,
            selected_project_path: None,
            launch_at_login: false,
        }
    }
}

fn settings_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(home).join(".slashpad");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("settings.json")
}

impl AppSettings {
    /// Load settings from disk, or return defaults if the file is
    /// missing or corrupt. Auto-detects `preferred_terminal` exactly
    /// once — on the first load where the key isn't already present
    /// in settings.json — and persists the detected value so later
    /// runs use the same choice without re-detecting. After that, a
    /// user's explicit pick from the Settings dropdown is never
    /// overridden (the key is present, so detection doesn't fire).
    ///
    /// Also scrubs any legacy plaintext `apiKey` left over from
    /// earlier versions: if that key is present in the raw JSON we
    /// rewrite the file without it. Users re-enter the key once and
    /// it's persisted to the OS keychain via the `secrets` module.
    pub fn load_or_default() -> Self {
        let path = settings_path();
        let raw = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<serde_json::Value>(&text).ok(),
            Err(_) => None,
        };

        let had_preferred_terminal = raw
            .as_ref()
            .and_then(|v| v.get("preferredTerminal"))
            .is_some();

        let had_legacy_api_key = raw
            .as_ref()
            .and_then(|v| v.get("apiKey"))
            .is_some();

        let mut settings: Self = raw
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        if !had_preferred_terminal {
            settings.preferred_terminal = PreferredTerminal::detect();
            if let Err(e) = settings.save() {
                eprintln!("[slashpad] failed to persist detected terminal: {e}");
            }
        }

        if had_legacy_api_key {
            // AppSettings no longer carries `apiKey`, so serializing
            // over the file drops the plaintext key. We intentionally
            // do NOT migrate it into the keychain — the user re-enters
            // it through the Settings UI.
            if let Err(e) = settings.save() {
                eprintln!("[slashpad] failed to scrub legacy apiKey: {e}");
            }
        }

        settings
    }

    pub fn save(&self) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string());
        std::fs::write(settings_path(), json)
    }
}
