---
name: release
description: Release a new version of Slashpad. Use when the user says "release", "cut a release", "bump the version", "ship it", or asks to publish a new version. Handles version bumping, committing, tagging, and creating a GitHub release with signed DMGs.
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

### 3.5. Generate release notes

GitHub's `--generate-notes` builds the body from merged PRs. Slashpad commits
directly to `main`, so auto-gen collapses to just a compare link. Write the
notes yourself instead.

1. Find the previous tag:
   ```bash
   PREV_TAG=$(gh release list --repo agencyenterprise/slashpad --limit 1 --json tagName -q '.[0].tagName')
   # fallback if gh is unreachable:
   # PREV_TAG=$(git describe --tags --abbrev=0 HEAD^)
   ```
2. Collect commits since that tag:
   ```bash
   git log "$PREV_TAG"..HEAD --pretty=format:'- %s (%h)'
   ```
3. Write the notes to `/tmp/slashpad-release-notes.md` in this shape:

   ```markdown
   ## Highlights

   - <one short, user-facing bullet per notable change, present tense>

   ## All changes

   - <commit subject> (<short sha>)
   - ...

   **Full Changelog**: https://github.com/agencyenterprise/slashpad/compare/<prev-tag>...v<new-version>
   ```

   Guidance for **Highlights**:
   - 2–5 bullets, framed for a user (what changed for them), not the raw commit subject.
   - Skip pure chore/version-bump commits.
   - For a **prerelease** with nothing user-visible yet, a single line like
     "Prerelease of X.Y.Z — testing <area>" is fine; omit the Highlights section.

### 4. Run the release script

For a **prerelease**:
```bash
./scripts/release.sh X.Y.Z-pre.N --prerelease --notes-file /tmp/slashpad-release-notes.md
```

For a **stable release**:
```bash
./scripts/release.sh X.Y.Z --notes-file /tmp/slashpad-release-notes.md
```

This script:
1. Creates a GitHub prerelease with the notes from `--notes-file` (falls back to GitHub auto-gen if omitted). All releases start as prereleases.
2. Waits for CI assets: DMGs and .app zips for both arches (~5-8 min)
3. For **stable releases only**: promotes the release from prerelease to full release so the in-app updater picks it up

**Note:** The release script will block for several minutes while GitHub Actions builds
the binaries and assembles the DMGs. This is expected — it polls until all assets appear.

### 5. Report

Print a summary:
- Version: X.Y.Z
- Release URL: `https://github.com/agencyenterprise/slashpad/releases/tag/vX.Y.Z`
- DMG downloads (Apple Silicon + Intel)
- For prereleases:
  - Note: Prerelease — the in-app updater will not surface it to stable builds. Download the DMG manually from the release page.
