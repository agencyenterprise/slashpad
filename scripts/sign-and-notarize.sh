#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/sign-and-notarize.sh [arch]
#   arch: aarch64 or x86_64 (default: detected from uname -m)
#
# Signs and notarizes Slashpad.app in the build/ directory.
# Must run AFTER build-app.sh and BEFORE create-dmg.sh.
#
# Required environment variables:
#   APPLE_CERTIFICATE_BASE64  - Base64-encoded .p12 certificate
#   APPLE_CERTIFICATE_PASSWORD - Password for the .p12
#   APPLE_ID                  - Apple ID email for notarytool
#   APPLE_ID_PASSWORD         - App-specific password for notarytool
#   APPLE_TEAM_ID             - Apple Developer Team ID
#
# The signing identity is "Developer ID Application: <name> (<team>)"
# which is extracted automatically from the imported certificate.

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

if [ ! -d "$APP_DIR" ]; then
    echo "App bundle not found at $APP_DIR" >&2
    echo "Run scripts/build-app.sh first" >&2
    exit 1
fi

# Verify required env vars.
for var in APPLE_CERTIFICATE_BASE64 APPLE_CERTIFICATE_PASSWORD APPLE_ID APPLE_ID_PASSWORD APPLE_TEAM_ID; do
    if [ -z "${!var:-}" ]; then
        echo "Missing required env var: $var" >&2
        exit 1
    fi
done

# ── 1. Import certificate into a temporary keychain ──────────────
KEYCHAIN_PATH="$RUNNER_TEMP/signing.keychain-db"
KEYCHAIN_PASSWORD="$(openssl rand -hex 16)"
P12_PATH="$RUNNER_TEMP/certificate.p12"

echo "==> Importing signing certificate"

# Decode the certificate.
echo "$APPLE_CERTIFICATE_BASE64" | base64 --decode > "$P12_PATH"

# Create and configure a temporary keychain.
security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
security set-keychain-settings -lut 21600 "$KEYCHAIN_PATH"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"

# Import the certificate.
security import "$P12_PATH" \
    -P "$APPLE_CERTIFICATE_PASSWORD" \
    -A \
    -t cert \
    -f pkcs12 \
    -k "$KEYCHAIN_PATH"

# Allow codesign to access the keychain without prompting.
security set-key-partition-list -S apple-tool:,apple:,codesign: \
    -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"

# Add the temporary keychain to the search list.
security list-keychains -d user -s "$KEYCHAIN_PATH" $(security list-keychains -d user | tr -d '"')

# Find the signing identity.
IDENTITY=$(security find-identity -v -p codesigning "$KEYCHAIN_PATH" \
    | grep "Developer ID Application" \
    | head -1 \
    | sed 's/.*"\(.*\)"/\1/')

if [ -z "$IDENTITY" ]; then
    echo "No 'Developer ID Application' identity found in certificate" >&2
    exit 1
fi

echo "    Signing identity: $IDENTITY"

# ── 2. Sign the .app bundle ─────────────────────────────────────
echo "==> Signing Slashpad.app"

ENTITLEMENTS="$REPO_ROOT/macos/entitlements.plist"

# Sign embedded binaries first (inside-out signing).
# The bundled bun binary needs JIT entitlements.
codesign --force --options runtime \
    --entitlements "$ENTITLEMENTS" \
    --sign "$IDENTITY" \
    "$APP_DIR/Contents/Resources/bin/bun"
echo "    Signed bun"

# Sign the main binary.
codesign --force --options runtime \
    --entitlements "$ENTITLEMENTS" \
    --sign "$IDENTITY" \
    "$APP_DIR/Contents/MacOS/slashpad"
echo "    Signed slashpad binary"

# Sign the entire .app bundle (covers Info.plist, resources, etc).
codesign --force --options runtime \
    --entitlements "$ENTITLEMENTS" \
    --sign "$IDENTITY" \
    "$APP_DIR"
echo "    Signed Slashpad.app"

# Verify the signature.
codesign --verify --deep --strict "$APP_DIR"
echo "    Signature verified"

# ── 3. Notarize ─────────────────────────────────────────────────
echo "==> Notarizing Slashpad.app"

# Create a zip for notarization submission (notarytool requires a zip/dmg/pkg).
NOTARIZE_ZIP="$BUILD_DIR/Slashpad-notarize.zip"
(cd "$BUILD_DIR" && zip -qr "Slashpad-notarize.zip" Slashpad.app)

xcrun notarytool submit "$NOTARIZE_ZIP" \
    --apple-id "$APPLE_ID" \
    --password "$APPLE_ID_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" \
    --wait
echo "    Notarization complete"

# Clean up the notarization zip.
rm -f "$NOTARIZE_ZIP"

# ── 4. Staple the notarization ticket ───────────────────────────
xcrun stapler staple "$APP_DIR"
echo "    Stapled notarization ticket"

# ── 5. Create the final zip (signed + notarized) ────────────────
ZIP_NAME="Slashpad-darwin-${ARCH}.zip"
(cd "$BUILD_DIR" && zip -qr "$ZIP_NAME" Slashpad.app)
echo "    Created $ZIP_NAME"

# ── 6. Clean up keychain ────────────────────────────────────────
security delete-keychain "$KEYCHAIN_PATH"
rm -f "$P12_PATH"

echo "==> Done: signed and notarized $BUILD_DIR/$ZIP_NAME"
