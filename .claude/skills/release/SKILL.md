---
name: release
description: Release a new version of Slashpad. Use when the user says "release", "cut a release", "bump the version", "ship it", or asks to publish a new version. Handles version bumping, committing, tagging, creating a GitHub release, and updating the Homebrew tap formula.
---

# Release

## Workflow

### 1. Determine the new version

Ask the user what kind of bump this is if not specified:
- **patch** (0.1.0 → 0.1.1): bug fixes — also promotes a prerelease to stable (0.1.1-pre.2 → 0.1.1)
- **minor** (0.1.0 → 0.2.0): new features
- **major** (0.1.0 → 1.0.0): breaking changes
- **pre** (0.1.0 → 0.1.1-pre.1): prerelease — bumps the pre counter if already a prerelease (0.1.1-pre.1 → 0.1.1-pre.2)

Read the current version from `Cargo.toml` line 3 and compute the new version.

**Version computation rules:**

Given current version `X.Y.Z` or `X.Y.Z-pre.N`:

| Bump    | From stable `X.Y.Z` | From prerelease `X.Y.Z-pre.N` |
|---------|---------------------|-------------------------------|
| `patch` | `X.Y.(Z+1)`        | `X.Y.Z` (drop the pre suffix) |
| `minor` | `X.(Y+1).0`        | `X.(Y+1).0`                   |
| `major` | `(X+1).0.0`        | `(X+1).0.0`                   |
| `pre`   | `X.Y.(Z+1)-pre.1`  | `X.Y.Z-pre.(N+1)`             |

### 2. Bump version in both manifests

Update the `version` field in both files — these must stay in sync:
- `Cargo.toml` (line 3): `version = "X.Y.Z"` or `version = "X.Y.Z-pre.N"`
- `package.json` (line 3): `"version": "X.Y.Z"` or `"version": "X.Y.Z-pre.N"`

### 3. Regenerate the lockfile and commit the version bump

Run `cargo check` so that `Cargo.lock` picks up the new version, then commit all three files:

```bash
cargo check
git add Cargo.toml Cargo.lock package.json
git commit -m "Bump version to X.Y.Z"
git push origin main
```

### 4. Run the release script

For a **prerelease**:
```bash
./scripts/release.sh X.Y.Z-pre.N --prerelease
```

For a **stable release**:
```bash
./scripts/release.sh X.Y.Z
```

This script:
1. Creates a GitHub prerelease with auto-generated notes (all releases start as prereleases)
2. Waits for the source tarball to become available and computes its SHA
3. Waits for CI assets: binaries (aarch64 + x86_64), DMGs, and .app zips (~5-8 min)
4. Computes the SHA-256 of both binaries
5. For **stable releases only**: updates `Formula/slashpad.rb` with all URLs and SHAs, clones `agencyenterprise/homebrew-tap`, copies the formula, commits and pushes, then promotes the release from prerelease to full release

**Note:** The release script will block for several minutes while GitHub Actions builds
the binaries and assembles the DMGs. This is expected — it polls until all assets appear.

### 5. Commit the updated formula (stable releases only)

Skip this step for prereleases — they don't update the Homebrew tap.

The release script updates `Formula/slashpad.rb` with the new URL and SHA. Commit it:

```bash
git add Formula/slashpad.rb
git commit -m "Update Homebrew formula for vX.Y.Z"
git push origin main
```

### 6. Report

Print a summary:
- Version: X.Y.Z
- Release URL: `https://github.com/agencyenterprise/slashpad/releases/tag/vX.Y.Z`
- DMG downloads (Apple Silicon + Intel)
- For stable releases:
  - Install command: `brew install agencyenterprise/tap/slashpad`
  - Upgrade command: `brew upgrade slashpad`
- For prereleases:
  - Note: Prerelease — not published to Homebrew. Download DMG from the release page.
