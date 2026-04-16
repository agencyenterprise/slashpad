//! Check GitHub for newer Slashpad releases.

use serde::Deserialize;

/// Minimal subset of the GitHub releases API response.
#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
}

/// The current compiled-in version (from Cargo.toml).
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Check the GitHub releases API for a newer version. Returns
/// `Some("x.y.z")` if a newer tag exists, `None` otherwise.
/// Swallows all errors (network, parse, etc.) — a failed check
/// is silently ignored so the app never blocks on this.
pub async fn check_for_update() -> Option<String> {
    let client = reqwest::Client::builder()
        .user_agent("slashpad-update-check")
        .build()
        .ok()?;

    let release: GitHubRelease = client
        .get("https://api.github.com/repos/agencyenterprise/slashpad/releases/latest")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    let latest = release.tag_name.strip_prefix('v').unwrap_or(&release.tag_name);

    if is_newer(latest, CURRENT) {
        Some(latest.to_string())
    } else {
        None
    }
}

/// Simple semver comparison: true when `latest` > `current`.
fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let l = parse(latest);
    let c = parse(current);
    for i in 0..l.len().max(c.len()) {
        let lv = l.get(i).copied().unwrap_or(0);
        let cv = c.get(i).copied().unwrap_or(0);
        if lv > cv {
            return true;
        }
        if lv < cv {
            return false;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison() {
        assert!(is_newer("0.2.0", "0.1.8"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.8", "0.1.8"));
        assert!(!is_newer("0.1.7", "0.1.8"));
    }
}
