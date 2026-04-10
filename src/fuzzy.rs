//! Fuzzy skill filter — replaces Fuse.js with nucleo-matcher.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};

use crate::state::Skill;

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
