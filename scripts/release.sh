#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/release.sh [version] [--prerelease]
# If no version is given, reads it from Cargo.toml.

VERSION=""
IS_PRERELEASE=false
NOTES_FILE=""

while [ $# -gt 0 ]; do
    case "$1" in
        --prerelease)
            IS_PRERELEASE=true
            shift
            ;;
        --notes-file)
            NOTES_FILE="${2:-}"
            shift 2
            ;;
        --notes-file=*)
            NOTES_FILE="${1#*=}"
            shift
            ;;
        *)
            if [ -z "$VERSION" ]; then
                VERSION="$1"
            fi
            shift
            ;;
    esac
done

if [ -z "$VERSION" ]; then
    VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
fi

if [ -n "$NOTES_FILE" ] && [ ! -f "$NOTES_FILE" ]; then
    echo "ERROR: --notes-file '$NOTES_FILE' does not exist" >&2
    exit 1
fi

TAG="v${VERSION}"
REPO="agencyenterprise/slashpad"

echo "==> Releasing ${TAG}"
if $IS_PRERELEASE; then
    echo "    (prerelease — in-app updater will ignore it for stable builds)"
fi

# ── 1. Create GitHub release as prerelease (also creates + pushes the tag)
# All releases start as prereleases so the in-app update checker doesn't see
# them until the CI-built assets are fully attached. Stable releases get
# promoted at the very end.
if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
    echo "    Release ${TAG} already exists, skipping"
else
    if [ -n "$NOTES_FILE" ]; then
        NOTES_ARG=(--notes-file "$NOTES_FILE")
    else
        NOTES_ARG=(--generate-notes)
    fi
    gh release create "$TAG" \
        --repo "$REPO" \
        --title "Slashpad ${TAG}" \
        "${NOTES_ARG[@]}" \
        --prerelease
    echo "    Created prerelease ${TAG}"
fi

# ── 2. Wait for CI-built assets to be attached ────────────────────
# The in-app updater downloads the .app zip (Slashpad-darwin-<arch>.zip);
# users download the signed DMG manually. Both arches of both must land
# before we consider the release ready.
echo "==> Waiting for CI assets (DMGs + .app zips, this may take a few minutes)..."
for i in $(seq 1 120); do
    ASSETS=$(gh release view "$TAG" --repo "$REPO" --json assets -q '.assets[].name' 2>/dev/null || true)
    if echo "$ASSETS" | grep -q "Slashpad-darwin-aarch64.zip" && \
       echo "$ASSETS" | grep -q "Slashpad-darwin-x86_64.zip" && \
       echo "$ASSETS" | grep -q "Slashpad-darwin-aarch64.dmg" && \
       echo "$ASSETS" | grep -q "Slashpad-darwin-x86_64.dmg"; then
        echo "    All assets found"
        break
    fi
    if [ "$i" -eq 120 ]; then
        echo "    ERROR: CI assets not attached after ~8 minutes" >&2
        echo "    Check https://github.com/${REPO}/actions for build status" >&2
        exit 1
    fi
    sleep 4
done

# ── 3. Promote to full release (stable releases only) ─────────────
# The in-app update checker ignores prereleases for stable builds
# (see src/updates.rs), so promote the release now that assets are live.
if $IS_PRERELEASE; then
    echo "==> Leaving ${TAG} as prerelease"
else
    echo "==> Promoting ${TAG} to full release..."
    # --latest explicitly pins this tag as the repo's "Latest" release
    # so /releases/latest/download/... URLs in the README always resolve
    # here. Without it, GitHub's default algorithm sometimes sticks on an
    # older tag that was pinned in the past.
    gh release edit "$TAG" --repo "$REPO" --prerelease=false --latest
    echo "    Done"
fi

echo ""
echo "==> Done! Release: https://github.com/${REPO}/releases/tag/${TAG}"
echo ""
echo "    DMG (Apple Silicon): https://github.com/${REPO}/releases/download/${TAG}/Slashpad-darwin-aarch64.dmg"
echo "    DMG (Intel):         https://github.com/${REPO}/releases/download/${TAG}/Slashpad-darwin-x86_64.dmg"
echo ""
if $IS_PRERELEASE; then
    echo "    (prerelease — download from the release page)"
fi
