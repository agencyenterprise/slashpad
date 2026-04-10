//! Skill loader + bundled-skill seeder.
//!
//! Skills live at `~/.launchpad/.claude/skills/<name>/SKILL.md`. A SKILL.md
//! file has a YAML frontmatter block. Mirrors `src_react_legacy/lib/skills.ts`.

use std::path::{Path, PathBuf};

use include_dir::{include_dir, Dir};
use serde::Deserialize;

use crate::state::Skill;

static SKILL_CREATOR_DIR: Dir<'_> =
    include_dir!("$CARGO_MANIFEST_DIR/bundled-skills/skill-creator");

fn skills_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".launchpad")
        .join(".claude")
        .join("skills")
}

#[derive(Debug, Deserialize)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(rename = "user-invocable", default)]
    user_invocable: Option<bool>,
}

fn parse_frontmatter(content: &str) -> Option<Frontmatter> {
    let stripped = content.strip_prefix("---\n")?;
    let end = stripped.find("\n---")?;
    let block = &stripped[..end];
    serde_yaml::from_str::<Frontmatter>(block).ok()
}

/// Load every user-invocable skill under `~/.launchpad/.claude/skills`.
pub fn load_skills() -> anyhow::Result<Vec<Skill>> {
    let root = skills_dir();
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for entry in std::fs::read_dir(&root)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_md = path.join("SKILL.md");
        let content = match std::fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let Some(fm) = parse_frontmatter(&content) else {
            continue;
        };
        if matches!(fm.user_invocable, Some(false)) {
            continue;
        }
        let (Some(name), Some(description)) = (fm.name, fm.description) else {
            continue;
        };
        out.push(Skill {
            name,
            description,
            path: skill_md.to_string_lossy().to_string(),
        });
    }
    Ok(out)
}

/// Seed `skill-creator` into the user's skills directory on first run.
pub fn seed_bundled_skills() -> std::io::Result<()> {
    let root = skills_dir();
    std::fs::create_dir_all(&root)?;
    let dest = root.join("skill-creator");
    if dest.exists() {
        return Ok(());
    }
    extract_dir(&SKILL_CREATOR_DIR, &dest)
}

fn extract_dir(dir: &Dir<'_>, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for file in dir.files() {
        if let Some(name) = file.path().file_name() {
            std::fs::write(dest.join(name), file.contents())?;
        }
    }
    for sub in dir.dirs() {
        if let Some(name) = sub.path().file_name() {
            extract_dir(sub, &dest.join(name))?;
        }
    }
    Ok(())
}
