//! Check GitHub for newer Slashpad releases.

use serde::Deserialize;

/// Minimal subset of the GitHub releases API response.
#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    prerelease: bool,
}

/// The current compiled-in version (from Cargo.toml).
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Check the GitHub releases API for a newer version. Returns
/// `Some("x.y.z")` if a newer tag exists, `None` otherwise.
/// Swallows all errors (network, parse, etc.) — a failed check
/// is silently ignored so the app never blocks on this.
///
/// Behaviour depends on whether the running binary is a prerelease:
/// - Prerelease build (e.g. `0.1.13-pre.1`): considers ALL releases,
///   so the user sees both newer prereleases and stable promotions.
/// - Stable build (e.g. `0.1.13`): only considers stable releases,
///   so users on the Homebrew tap never get pointed at a prerelease.
pub async fn check_for_update() -> Option<String> {
    let client = reqwest::Client::builder()
        .user_agent("slashpad-update-check")
        .build()
        .ok()?;

    let releases: Vec<GitHubRelease> = client
        .get("https://api.github.com/repos/agencyenterprise/slashpad/releases?per_page=20")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    let current_is_pre = CURRENT.contains("-pre");

    // Find the newest release that this build should care about.
    let candidate = if current_is_pre {
        // Prerelease builds see everything.
        releases.first()
    } else {
        // Stable builds only see stable releases.
        releases.iter().find(|r| !r.prerelease)
    }?;

    let latest = candidate
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&candidate.tag_name);

    if is_newer(latest, CURRENT) {
        Some(latest.to_string())
    } else {
        None
    }
}

/// Semver comparison that handles pre-release suffixes.
///
/// Ordering rules (matching semver spec):
/// - `0.1.13-pre.1 < 0.1.13-pre.2 < 0.1.13`
/// - A pre-release version is always less than its release counterpart.
/// - Pre-release precedence is compared numerically by the pre counter.
///
/// Returns true when `latest` > `current`.
fn is_newer(latest: &str, current: &str) -> bool {
    let (l_base, l_pre) = parse_version(latest);
    let (c_base, c_pre) = parse_version(current);

    // Compare base version parts first.
    for i in 0..l_base.len().max(c_base.len()) {
        let lv = l_base.get(i).copied().unwrap_or(0);
        let cv = c_base.get(i).copied().unwrap_or(0);
        if lv > cv {
            return true;
        }
        if lv < cv {
            return false;
        }
    }

    // Base versions are equal — compare pre-release.
    match (l_pre, c_pre) {
        // Both stable with same base → equal, not newer.
        (None, None) => false,
        // latest is stable, current is pre with same base → latest is newer
        // (e.g. 0.1.13 > 0.1.13-pre.2)
        (None, Some(_)) => true,
        // latest is pre, current is stable with same base → latest is older
        // (e.g. 0.1.13-pre.2 < 0.1.13)
        (Some(_), None) => false,
        // Both pre with same base → higher pre number wins.
        (Some(lp), Some(cp)) => lp > cp,
    }
}

/// Parse "X.Y.Z" or "X.Y.Z-pre.N" into (base parts, optional pre number).
fn parse_version(s: &str) -> (Vec<u64>, Option<u64>) {
    let (base, pre) = if let Some((b, p)) = s.split_once("-pre.") {
        (b, p.parse::<u64>().ok())
    } else {
        (s, None)
    };

    let parts: Vec<u64> = base
        .split('.')
        .map(|p| p.parse::<u64>().unwrap_or(0))
        .collect();

    (parts, pre)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_comparisons() {
        assert!(is_newer("0.2.0", "0.1.8"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.8", "0.1.8"));
        assert!(!is_newer("0.1.7", "0.1.8"));
    }

    #[test]
    fn prerelease_comparisons() {
        // Higher pre number is newer
        assert!(is_newer("0.1.13-pre.2", "0.1.13-pre.1"));
        assert!(!is_newer("0.1.13-pre.1", "0.1.13-pre.2"));
        // Same pre is not newer
        assert!(!is_newer("0.1.13-pre.1", "0.1.13-pre.1"));
    }

    #[test]
    fn stable_vs_prerelease() {
        // Stable release is newer than its prerelease
        assert!(is_newer("0.1.13", "0.1.13-pre.2"));
        // Prerelease is older than its stable release
        assert!(!is_newer("0.1.13-pre.2", "0.1.13"));
    }

    #[test]
    fn cross_version_prerelease() {
        // A prerelease for a higher version is newer than a lower stable
        assert!(is_newer("0.2.0-pre.1", "0.1.13"));
        // A prerelease for a lower version is older than a higher stable
        assert!(!is_newer("0.1.12-pre.3", "0.1.13"));
    }
}
