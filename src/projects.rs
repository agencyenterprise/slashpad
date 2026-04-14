//! Known-projects loader — enumerates the directories Claude Code has
//! already been run in, by reading `~/.claude/projects/`.
//!
//! The Claude Code CLI writes one subdirectory per `cwd` it's been run
//! in, naming the subdir by replacing `/` with `-` in the absolute
//! path (e.g. `/Users/alice/dev/foo` → `-Users-alice-dev-foo`). That
//! mangling is ambiguous for directories whose real name contains a
//! `-`, so we decode with a straight `- → /` substitution and then
//! drop any entries whose decoded path doesn't exist on disk — the
//! ambiguous cases self-filter.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A directory Claude Code has been run in.
#[derive(Debug, Clone)]
pub struct ProjectInfo {
    /// Decoded absolute path on disk.
    pub path: PathBuf,
    /// Tilde-abbreviated display string, also used as the fuzzy-match
    /// haystack so the user can search by the form they see.
    pub display: String,
    /// Modification time of the `~/.claude/projects/<mangled>` subdir,
    /// used as a cheap proxy for "most recently active" when sorting.
    pub last_modified: SystemTime,
    /// True for the built-in `~/.launchpad` entry — the project
    /// Launchpad ran in before the user could pick one. Pinned first
    /// in the unfiltered list and rendered with a "default" label so
    /// the user always has a way back to it.
    pub is_default: bool,
}

/// Scan `~/.claude/projects/` and return the list of still-existing
/// project directories. Runs the blocking `std::fs` calls on a
/// `spawn_blocking` worker so the async caller doesn't stall.
pub async fn list_known() -> anyhow::Result<Vec<ProjectInfo>> {
    tokio::task::spawn_blocking(scan_sync).await?
}

fn scan_sync() -> anyhow::Result<Vec<ProjectInfo>> {
    let home = std::env::var("HOME")?;
    let root = PathBuf::from(&home).join(".claude").join("projects");
    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        // No `~/.claude/projects/` yet → no known projects. Return
        // empty rather than erroring so the picker just shows its
        // empty state.
        Err(_) => return Ok(Vec::new()),
    };

    let mut out: Vec<ProjectInfo> = Vec::new();
    for entry in entries.flatten() {
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if !file_type.is_dir() {
            continue;
        }
        let name = match entry.file_name().into_string() {
            Ok(s) => s,
            Err(_) => continue,
        };
        let Some(decoded) = decode_mangled(&name) else {
            continue;
        };
        // Drop entries whose decoded path is gone from disk — covers
        // both truly-deleted projects and the rare ambiguity case
        // where a dirname with `-` in it decodes to a nonsense path.
        if !decoded.is_dir() {
            continue;
        }
        let last_modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let display = display_path(&decoded, &home);
        out.push(ProjectInfo {
            path: decoded,
            display,
            last_modified,
            is_default: false,
        });
    }

    // Most-recently-active first so an empty-query picker lands the
    // user on a likely target immediately.
    out.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));

    // Pin `~/.launchpad` — the pre-project-picker hardcoded cwd — as
    // the first entry so the user always has a way back to the
    // default. Flag it so the picker can render a label. If the scan
    // already surfaced `~/.launchpad`, promote that entry in place;
    // otherwise synthesize one even if the dir hasn't been run yet
    // (seed_default_claude_md creates it on startup, so it should
    // always exist — but fall back to the path as-is if not).
    let launchpad = PathBuf::from(&home).join(".launchpad");
    if let Some(pos) = out.iter().position(|p| p.path == launchpad) {
        let mut existing = out.remove(pos);
        existing.is_default = true;
        out.insert(0, existing);
    } else {
        let last_modified = std::fs::metadata(&launchpad)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        out.insert(
            0,
            ProjectInfo {
                display: display_path(&launchpad, &home),
                path: launchpad,
                last_modified,
                is_default: true,
            },
        );
    }

    Ok(out)
}

/// Reverse the Claude Code dirname mangling. The mangled form prefixes
/// an absolute path with `-` (standing in for the leading `/`) and
/// replaces every subsequent `/` with `-`, so we flip each `-` back
/// to `/`. Returns `None` for empty or non-`-`-prefixed names.
fn decode_mangled(name: &str) -> Option<PathBuf> {
    if !name.starts_with('-') {
        return None;
    }
    Some(PathBuf::from(name.replace('-', "/")))
}

/// Tilde-abbreviate `$HOME` in a path for display. Duplicates the
/// helper in `app.rs` rather than depending on it — keeps the module
/// graph acyclic and the function is four lines.
fn display_path(path: &Path, home: &str) -> String {
    let s = path.to_string_lossy();
    if !home.is_empty() {
        if let Some(rest) = s.strip_prefix(home) {
            return format!("~{}", rest);
        }
    }
    s.into_owned()
}
