---
name: release
description: Release a new version of Slashpad. Use when the user says "release", "cut a release", "bump the version", "ship it", or asks to publish a new version. Handles version bumping, committing, tagging, creating a GitHub prerelease, and updating the Homebrew tap formula.
---

# Release

## Workflow

### 1. Determine the new version

Ask the user what kind of bump this is if not specified:
- **patch** (0.1.0 → 0.1.1): bug fixes
- **minor** (0.1.0 → 0.2.0): new features
- **major** (0.1.0 → 1.0.0): breaking changes

Read the current version from `Cargo.toml` line 3 and compute the new version.

### 2. Bump version in both manifests

Update the `version` field in both files — these must stay in sync:
- `Cargo.toml` (line 3): `version = "X.Y.Z"`
- `package.json` (line 3): `"version": "X.Y.Z"`

### 3. Regenerate the lockfile and commit the version bump

Run `cargo check` so that `Cargo.lock` picks up the new version, then commit all three files:

```bash
cargo check
git add Cargo.toml Cargo.lock package.json
git commit -m "Bump version to X.Y.Z"
git push origin main
```

### 4. Run the release script

```bash
./scripts/release.sh X.Y.Z
```

This script:
1. Creates a GitHub prerelease with auto-generated notes
2. Waits for the tarball to become available
3. Computes the SHA-256 of the tarball
4. Updates `Formula/slashpad.rb` in this repo
5. Clones `agencyenterprise/homebrew-tap`, copies the formula, commits and pushes

### 5. Commit the updated formula

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
- Install command: `brew install agencyenterprise/tap/slashpad`
- Upgrade command: `brew upgrade slashpad`
