#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/release.sh [version] [--prerelease]
# If no version is given, reads it from Cargo.toml.

VERSION="${1:-}"
IS_PRERELEASE=false

for arg in "$@"; do
    if [ "$arg" = "--prerelease" ]; then
        IS_PRERELEASE=true
    fi
done

if [ -z "$VERSION" ] || [ "$VERSION" = "--prerelease" ]; then
    VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
fi

TAG="v${VERSION}"
REPO="agencyenterprise/slashpad"
TAP_REPO="agencyenterprise/homebrew-tap"
TARBALL_URL="https://github.com/${REPO}/archive/refs/tags/${TAG}.tar.gz"
BINARY_AARCH64_URL="https://github.com/${REPO}/releases/download/${TAG}/slashpad-darwin-aarch64"
BINARY_X86_64_URL="https://github.com/${REPO}/releases/download/${TAG}/slashpad-darwin-x86_64"

echo "==> Releasing ${TAG}"
if $IS_PRERELEASE; then
    echo "    (prerelease — Homebrew tap will NOT be updated)"
fi

# ── 1. Create GitHub release (also creates + pushes the tag) ──────
if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
    echo "    Release ${TAG} already exists, skipping"
else
    RELEASE_FLAGS=(
        --repo "$REPO"
        --title "Slashpad ${TAG}"
        --generate-notes
    )
    if $IS_PRERELEASE; then
        RELEASE_FLAGS+=(--prerelease)
    fi
    gh release create "$TAG" "${RELEASE_FLAGS[@]}"
    echo "    Created release ${TAG}"
fi

# ── 2. Wait for the source tarball ────────────────────────────────
echo "==> Waiting for GitHub to serve the tarball..."
for i in $(seq 1 30); do
    if curl -sfIL "$TARBALL_URL" >/dev/null 2>&1; then
        break
    fi
    if [ "$i" -eq 30 ]; then
        echo "    ERROR: tarball not available after 60s" >&2
        exit 1
    fi
    sleep 2
done
TARBALL_SHA=$(curl -sL "$TARBALL_URL" | shasum -a 256 | awk '{print $1}')
echo "    tarball sha256: ${TARBALL_SHA}"

# ── 3. Wait for CI-built binaries to be attached ──────────────────
echo "==> Waiting for CI binaries (this may take a few minutes)..."
for i in $(seq 1 90); do
    ASSETS=$(gh release view "$TAG" --repo "$REPO" --json assets -q '.assets[].name' 2>/dev/null || true)
    if echo "$ASSETS" | grep -q "slashpad-darwin-aarch64" && \
       echo "$ASSETS" | grep -q "slashpad-darwin-x86_64"; then
        echo "    Both binaries found"
        break
    fi
    if [ "$i" -eq 90 ]; then
        echo "    ERROR: CI binaries not attached after ~6 minutes" >&2
        echo "    Check https://github.com/${REPO}/actions for build status" >&2
        exit 1
    fi
    sleep 4
done

# ── 4. Compute binary SHAs ────────────────────────────────────────
AARCH64_SHA=$(curl -sL "$BINARY_AARCH64_URL" | shasum -a 256 | awk '{print $1}')
X86_64_SHA=$(curl -sL "$BINARY_X86_64_URL" | shasum -a 256 | awk '{print $1}')
echo "    aarch64 sha256: ${AARCH64_SHA}"
echo "    x86_64  sha256: ${X86_64_SHA}"

# ── 5. Update Homebrew formula (stable releases only) ─────────────
if $IS_PRERELEASE; then
    echo "==> Skipping Homebrew formula update (prerelease)"
else
    # Source tarball URL + SHA (lines 4-5)
    sed -i '' "4s|url \".*\"|url \"${TARBALL_URL}\"|" Formula/slashpad.rb
    sed -i '' "5s|sha256 \".*\"|sha256 \"${TARBALL_SHA}\"|" Formula/slashpad.rb

    # Binary resource URLs + SHAs
    sed -i '' "s|releases/download/v[^/]*/slashpad-darwin-aarch64|releases/download/${TAG}/slashpad-darwin-aarch64|" Formula/slashpad.rb
    sed -i '' "s|releases/download/v[^/]*/slashpad-darwin-x86_64|releases/download/${TAG}/slashpad-darwin-x86_64|" Formula/slashpad.rb
    sed -i '' "/slashpad-darwin-aarch64/{n;s|sha256 \".*\"|sha256 \"${AARCH64_SHA}\"|;}" Formula/slashpad.rb
    sed -i '' "/slashpad-darwin-x86_64/{n;s|sha256 \".*\"|sha256 \"${X86_64_SHA}\"|;}" Formula/slashpad.rb

    echo "==> Updated Formula/slashpad.rb"

    # ── 6. Clone tap repo, update formula, push ───────────────────
    TMPDIR=$(mktemp -d)
    trap 'rm -rf "$TMPDIR"' EXIT

    echo "==> Cloning ${TAP_REPO}..."
    gh repo clone "$TAP_REPO" "$TMPDIR" -- -q

    mkdir -p "$TMPDIR/Formula"
    cp Formula/slashpad.rb "$TMPDIR/Formula/slashpad.rb"

    cd "$TMPDIR"
    git add Formula/slashpad.rb
    if git diff --cached --quiet; then
        echo "    Tap formula already up to date"
    else
        git commit -m "slashpad ${TAG}"
        git push origin main
        echo "    Pushed formula to ${TAP_REPO}"
    fi
    cd - >/dev/null
fi

echo ""
echo "==> Done! Release: https://github.com/${REPO}/releases/tag/${TAG}"
if $IS_PRERELEASE; then
    echo "    (prerelease — download binaries from the release page)"
else
    echo "    Install: brew install agencyenterprise/tap/slashpad"
    echo "    Upgrade: brew upgrade slashpad"
fi
