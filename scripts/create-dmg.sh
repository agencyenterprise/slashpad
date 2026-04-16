#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/create-dmg.sh [arch]
#   arch: aarch64 or x86_64 (default: detected from uname -m)
#
# Creates a DMG from the .app bundle in build/Slashpad.app.
# Requires `create-dmg` (brew install create-dmg).

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ARCH="${1:-}"
if [ -z "$ARCH" ]; then
    MACHINE=$(uname -m)
    case "$MACHINE" in
        arm64)  ARCH="aarch64" ;;
        x86_64) ARCH="x86_64" ;;
        *)      echo "Unknown machine: $MACHINE" >&2; exit 1 ;;
    esac
fi

BUILD_DIR="$REPO_ROOT/build"
APP_DIR="$BUILD_DIR/Slashpad.app"
DMG_NAME="Slashpad-darwin-${ARCH}.dmg"

if [ ! -d "$APP_DIR" ]; then
    echo "App bundle not found at $APP_DIR" >&2
    echo "Run scripts/build-app.sh first" >&2
    exit 1
fi

# Remove previous DMG if it exists (create-dmg fails otherwise).
rm -f "$BUILD_DIR/$DMG_NAME"

echo "==> Creating $DMG_NAME"

create-dmg \
    --volname "Slashpad" \
    --window-pos 200 120 \
    --window-size 600 400 \
    --icon-size 100 \
    --icon "Slashpad.app" 150 190 \
    --app-drop-link 450 190 \
    --no-internet-enable \
    "$BUILD_DIR/$DMG_NAME" \
    "$APP_DIR"

echo "==> Done: $BUILD_DIR/$DMG_NAME"
