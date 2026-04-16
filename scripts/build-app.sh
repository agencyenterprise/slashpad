#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/build-app.sh [target] [arch]
#   target: Rust target triple (default: current host)
#   arch:   aarch64 or x86_64 (default: detected from target or uname -m)
#
# Assembles Slashpad.app from a pre-built release binary.
# Expects `cargo build --release --target $TARGET` to have already run.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

TARGET="${1:-}"
ARCH="${2:-}"

# Default target to the host triple.
if [ -z "$TARGET" ]; then
    MACHINE=$(uname -m)
    if [ "$MACHINE" = "arm64" ]; then
        TARGET="aarch64-apple-darwin"
    else
        TARGET="x86_64-apple-darwin"
    fi
fi

# Derive arch from target if not given.
if [ -z "$ARCH" ]; then
    case "$TARGET" in
        aarch64-*) ARCH="aarch64" ;;
        x86_64-*)  ARCH="x86_64" ;;
        *)         echo "Cannot determine arch from target: $TARGET" >&2; exit 1 ;;
    esac
fi

# Map arch to bun's naming convention.
case "$ARCH" in
    aarch64) BUN_ARCH="aarch64" ;;
    x86_64)  BUN_ARCH="x64" ;;
    *)       echo "Unknown arch: $ARCH" >&2; exit 1 ;;
esac

BUN_VERSION="1.3.12"
VERSION=$(grep '^version' "$REPO_ROOT/Cargo.toml" | head -1 | sed 's/.*"\(.*\)"/\1/')
BINARY="$REPO_ROOT/target/$TARGET/release/slashpad"
# Also check the plain release dir (when built without --target).
if [ ! -f "$BINARY" ]; then
    BINARY="$REPO_ROOT/target/release/slashpad"
fi

if [ ! -f "$BINARY" ]; then
    echo "Binary not found. Run one of:" >&2
    echo "  cargo build --release --target $TARGET" >&2
    echo "  cargo build --release" >&2
    exit 1
fi

BUILD_DIR="$REPO_ROOT/build"
APP_DIR="$BUILD_DIR/Slashpad.app"

echo "==> Assembling Slashpad.app (v${VERSION}, ${ARCH})"

# Clean previous build.
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources/agent"
mkdir -p "$APP_DIR/Contents/Resources/bin"

# ── 1. Binary ─────────────────────────────────────────────────────
cp "$BINARY" "$APP_DIR/Contents/MacOS/slashpad"
chmod 755 "$APP_DIR/Contents/MacOS/slashpad"
echo "    Copied binary"

# ── 2. Info.plist (stamp version) ─────────────────────────────────
sed "s/__VERSION__/$VERSION/g" "$REPO_ROOT/macos/Info.plist" \
    > "$APP_DIR/Contents/Info.plist"
echo "    Stamped Info.plist with v${VERSION}"

# ── 3. Icon ───────────────────────────────────────────────────────
cp "$REPO_ROOT/icons/icon.icns" "$APP_DIR/Contents/Resources/icon.icns"
echo "    Copied icon"

# ── 4. Bun runtime ───────────────────────────────────────────────
BUN_URL="https://github.com/oven-sh/bun/releases/download/bun-v${BUN_VERSION}/bun-darwin-${BUN_ARCH}.zip"
BUN_TMP=$(mktemp -d)
echo "    Downloading bun v${BUN_VERSION} (${BUN_ARCH})..."
curl -sL "$BUN_URL" -o "$BUN_TMP/bun.zip"
unzip -q "$BUN_TMP/bun.zip" -d "$BUN_TMP"
# The zip contains a directory like bun-darwin-aarch64/bun
cp "$BUN_TMP"/bun-*/bun "$APP_DIR/Contents/Resources/bin/bun"
chmod 755 "$APP_DIR/Contents/Resources/bin/bun"
rm -rf "$BUN_TMP"
echo "    Bundled bun"

# ── 5. Sidecar (agent + dependencies) ────────────────────────────
cp "$REPO_ROOT/agent/runner.mjs" "$APP_DIR/Contents/Resources/agent/runner.mjs"
cp "$REPO_ROOT/package.json" "$APP_DIR/Contents/Resources/package.json"
echo "    Copied sidecar files"

# Install production dependencies. Use host bun/node for `install` since
# the bundled bun may be cross-compiled (node_modules are arch-independent).
if command -v bun >/dev/null 2>&1; then
    (cd "$APP_DIR/Contents/Resources" && bun install --production 2>&1 | tail -1)
elif command -v npm >/dev/null 2>&1; then
    (cd "$APP_DIR/Contents/Resources" && npm install --production 2>&1 | tail -1)
else
    # Last resort: try the bundled bun (only works if same arch as host).
    (cd "$APP_DIR/Contents/Resources" && "$APP_DIR/Contents/Resources/bin/bun" install --production 2>&1 | tail -1)
fi
echo "    Installed node_modules"

# ── 6. Zip for upload ────────────────────────────────────────────
ZIP_NAME="Slashpad-darwin-${ARCH}.zip"
(cd "$BUILD_DIR" && zip -qr "$ZIP_NAME" Slashpad.app)
echo "    Created $ZIP_NAME"

echo "==> Done: $BUILD_DIR/$ZIP_NAME"
