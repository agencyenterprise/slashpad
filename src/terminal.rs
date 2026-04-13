//! Launch an external terminal emulator to resume a Claude Code session.
//!
//! Called from the Cmd+T handler in `app.rs` when the user wants to move
//! a chat from the palette into a full terminal. The target command is
//! always `cd <cwd> && claude --resume <session-id>` (or equivalent) —
//! the sidecar runs in `~/.launchpad`, so resuming there finds the same
//! project context Claude Code uses for its own session storage.
//!
//! Launches are spawn-and-forget (no `.wait()`): we return `io::Result`
//! from the `.spawn()` call itself, and the caller just logs on error.

use std::io;
use std::process::Command;

use crate::settings::PreferredTerminal;

/// Open a new terminal window running `claude --resume <session_id>` in
/// `cwd`. Returns the spawn error if the terminal binary (or
/// `osascript` for the AppleScript variants) couldn't be started.
pub fn open_claude_resume(
    term: PreferredTerminal,
    cwd: &str,
    session_id: &str,
) -> io::Result<()> {
    match term {
        PreferredTerminal::Terminal => spawn_terminal_app(cwd, session_id),
        PreferredTerminal::ITerm => spawn_iterm(cwd, session_id),
        PreferredTerminal::Ghostty => spawn_ghostty(cwd, session_id),
        PreferredTerminal::Warp => spawn_warp(cwd),
        PreferredTerminal::Alacritty => spawn_alacritty(cwd, session_id),
        PreferredTerminal::Kitty => spawn_kitty(cwd, session_id),
        PreferredTerminal::WezTerm => spawn_wezterm(cwd, session_id),
    }
}

/// Escape a string for embedding inside a single-quoted AppleScript
/// literal. AppleScript strings are actually double-quoted, so we
/// escape `"` → `\"` and `\` → `\\`. Session IDs are UUID-shaped so
/// this mostly matters for `cwd` (which can contain spaces).
fn escape_applescript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn spawn_terminal_app(cwd: &str, session_id: &str) -> io::Result<()> {
    // `do script` opens a new Terminal window running the given shell
    // command; `activate` brings Terminal.app to the front so focus
    // lands in the new window instead of leaving the palette's window
    // frontmost.
    let cmd = format!(
        "cd \"{}\" && claude --resume \"{}\"",
        escape_applescript(cwd),
        escape_applescript(session_id),
    );
    let script = format!(
        "tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell",
        cmd.replace('\\', "\\\\").replace('"', "\\\""),
    );
    Command::new("osascript")
        .arg("-e")
        .arg(script)
        .spawn()
        .map(|_| ())
}

fn spawn_iterm(cwd: &str, session_id: &str) -> io::Result<()> {
    // iTerm2's AppleScript dictionary exposes `create window with
    // default profile command`, which launches a new window that runs
    // the given shell command. Wrap it in `sh -c` so the `cd && ...`
    // sequence works.
    let inner = format!(
        "cd \"{}\" && claude --resume \"{}\"",
        escape_applescript(cwd),
        escape_applescript(session_id),
    );
    let sh_cmd = format!("sh -c '{}'", inner.replace('\'', "'\\''"));
    let script = format!(
        "tell application \"iTerm\"\nactivate\ncreate window with default profile command \"{}\"\nend tell",
        sh_cmd.replace('\\', "\\\\").replace('"', "\\\""),
    );
    Command::new("osascript")
        .arg("-e")
        .arg(script)
        .spawn()
        .map(|_| ())
}

fn spawn_ghostty(cwd: &str, session_id: &str) -> io::Result<()> {
    // Ghostty on macOS explicitly does NOT support launching the
    // emulator from the CLI — `ghostty --help` directs callers to
    // `open -na Ghostty.app --args ...`. The `-n` flag forces a new
    // process so `--args` are honored even when Ghostty is already
    // running (otherwise AppKit reuses the existing instance and
    // drops the args, which is the bug that showed up in testing).
    //
    // We wrap the `claude --resume <id>` invocation inside `sh -c`
    // so Ghostty's own arg parser doesn't try to interpret
    // `--resume` as a Ghostty config flag — everything after `-e`
    // gets consumed as the command argv, and `sh -c` hides the
    // inner flags entirely. The inner command also prepends a `cd`
    // because Ghostty's `--working-directory` interacts awkwardly
    // with `-e` on some configs, and an explicit `cd` is free
    // insurance.
    let inner = format!(
        "cd {} && exec claude --resume {}",
        shell_escape(cwd),
        shell_escape(session_id),
    );
    Command::new("open")
        .arg("-na")
        .arg("Ghostty.app")
        .arg("--args")
        .arg("-e")
        .arg("sh")
        .arg("-c")
        .arg(inner)
        .spawn()
        .map(|_| ())
}

/// Wrap `s` in single quotes, escaping any embedded single quotes
/// the POSIX way: `'\''`. Safe for use inside a `sh -c` command
/// string.
fn shell_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

fn spawn_warp(cwd: &str) -> io::Result<()> {
    // Warp's URL scheme opens a new tab at the given path, but doesn't
    // accept an initial command. Warp users run `claude --resume <id>`
    // themselves once the tab opens. The session id is still surfaced
    // to the user via the keyhints bar label before they press ⌘T, so
    // this remains a useful shortcut.
    Command::new("open")
        .arg(format!(
            "warp://action/new_tab?path={}",
            urlencode(cwd),
        ))
        .spawn()
        .map(|_| ())
}

fn spawn_alacritty(cwd: &str, session_id: &str) -> io::Result<()> {
    Command::new("alacritty")
        .arg("--working-directory")
        .arg(cwd)
        .arg("-e")
        .arg("claude")
        .arg("--resume")
        .arg(session_id)
        .spawn()
        .map(|_| ())
}

fn spawn_kitty(cwd: &str, session_id: &str) -> io::Result<()> {
    Command::new("kitty")
        .arg("--directory")
        .arg(cwd)
        .arg("claude")
        .arg("--resume")
        .arg(session_id)
        .spawn()
        .map(|_| ())
}

fn spawn_wezterm(cwd: &str, session_id: &str) -> io::Result<()> {
    Command::new("wezterm")
        .arg("start")
        .arg("--cwd")
        .arg(cwd)
        .arg("--")
        .arg("claude")
        .arg("--resume")
        .arg(session_id)
        .spawn()
        .map(|_| ())
}

/// Minimal percent-encoding for path components that go into a URL
/// query string. Only encodes the characters that cause trouble in
/// the Warp scheme; everything else passes through unchanged.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
