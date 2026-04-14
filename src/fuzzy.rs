//! Fuzzy skill filter — replaces Fuse.js with nucleo-matcher.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};

use crate::projects::ProjectInfo;
use crate::state::{SessionInfo, Skill};

/// Filter and rank skills by fuzzy match against the query. Matches name and
/// description; ranks by name match first.
pub fn filter_skills(skills: &[Skill], query: &str) -> Vec<Skill> {
    if query.is_empty() {
        return skills.to_vec();
    }

    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);

    let mut scored: Vec<(u32, Skill)> = Vec::new();
    let mut buf = Vec::new();
    for skill in skills {
        buf.clear();
        let name_score =
            pattern.score(nucleo_matcher::Utf32Str::new(&skill.name, &mut buf), &mut matcher);
        buf.clear();
        let desc_score = pattern.score(
            nucleo_matcher::Utf32Str::new(&skill.description, &mut buf),
            &mut matcher,
        );
        let score = match (name_score, desc_score) {
            (Some(n), Some(d)) => n.saturating_add(d / 2),
            (Some(n), None) => n,
            (None, Some(d)) => d / 2,
            (None, None) => continue,
        };
        scored.push((score, skill.clone()));
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, s)| s).collect()
}

/// Filter and rank past sessions by fuzzy match. Matches the session
/// summary (primary) and first-prompt text (secondary, half-weighted —
/// mirrors the name/description weighting in `filter_skills`).
pub fn filter_sessions(sessions: &[SessionInfo], query: &str) -> Vec<SessionInfo> {
    if query.is_empty() {
        return sessions.to_vec();
    }

    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);

    let mut scored: Vec<(u32, SessionInfo)> = Vec::new();
    let mut buf = Vec::new();
    for session in sessions {
        buf.clear();
        let summary_score = pattern.score(
            nucleo_matcher::Utf32Str::new(&session.summary, &mut buf),
            &mut matcher,
        );
        let prompt_score = match session.first_prompt.as_deref() {
            Some(p) => {
                buf.clear();
                pattern.score(nucleo_matcher::Utf32Str::new(p, &mut buf), &mut matcher)
            }
            None => None,
        };
        let score = match (summary_score, prompt_score) {
            (Some(s), Some(p)) => s.saturating_add(p / 2),
            (Some(s), None) => s,
            (None, Some(p)) => p / 2,
            (None, None) => continue,
        };
        scored.push((score, session.clone()));
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, s)| s).collect()
}

/// Filter and rank known projects by fuzzy match against the query.
/// Scores against the tilde-abbreviated display string — that's the
/// form the user sees in the picker list, so it's also the form
/// they'll type fragments of.
pub fn filter_projects(projects: &[ProjectInfo], query: &str) -> Vec<ProjectInfo> {
    if query.is_empty() {
        return projects.to_vec();
    }

    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);

    let mut scored: Vec<(u32, ProjectInfo)> = Vec::new();
    let mut buf = Vec::new();
    for project in projects {
        buf.clear();
        let score = pattern.score(
            nucleo_matcher::Utf32Str::new(&project.display, &mut buf),
            &mut matcher,
        );
        if let Some(s) = score {
            scored.push((s, project.clone()));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, p)| p).collect()
}

/// Fuzzy-match a single haystack against a query. Returns `None` when the
/// query doesn't match. Convenience for callers that need to filter
/// something other than `Skill` / `SessionInfo` (e.g. active chat titles).
pub fn fuzzy_score(haystack: &str, query: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(0);
    }
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut buf = Vec::new();
    pattern.score(nucleo_matcher::Utf32Str::new(haystack, &mut buf), &mut matcher)
}
