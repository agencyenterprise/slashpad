#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/release.sh [version]
# If no version is given, reads it from Cargo.toml.

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
    VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
fi

TAG="v${VERSION}"
REPO="agencyenterprise/slashpad"
TAP_REPO="agencyenterprise/homebrew-tap"
TARBALL_URL="https://github.com/${REPO}/archive/refs/tags/${TAG}.tar.gz"

echo "==> Releasing ${TAG}"

# ── 1. Create GitHub prerelease (also creates + pushes the tag) ─────
if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
    echo "    Release ${TAG} already exists, skipping"
else
    gh release create "$TAG" \
        --repo "$REPO" \
        --title "Slashpad ${TAG}" \
        --generate-notes \
        --prerelease
    echo "    Created prerelease ${TAG}"
fi

# ── 2. Wait for the tarball to become available ─────────────────────
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

# ── 3. Compute SHA-256 ─────────────────────────────────────────────
SHA=$(curl -sL "$TARBALL_URL" | shasum -a 256 | awk '{print $1}')
echo "    sha256: ${SHA}"

# ── 4. Update the formula in this repo (reference copy) ────────────
# Only replace the top-level url/sha256 (lines 4-5), not resource blocks.
sed -i '' "4s|url \".*\"|url \"${TARBALL_URL}\"|" Formula/slashpad.rb
sed -i '' "5s|sha256 \".*\"|sha256 \"${SHA}\"|" Formula/slashpad.rb
echo "==> Updated Formula/slashpad.rb"

# ── 5. Clone tap repo, update formula, push ────────────────────────
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

echo ""
echo "==> Done! Users can install with:"
echo "    brew install agencyenterprise/tap/slashpad"
