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

# Import the .p12 (leaf cert + private key).
security import "$P12_PATH" \
    -P "$APPLE_CERTIFICATE_PASSWORD" \
    -A \
    -t cert \
    -f pkcs12 \
    -k "$KEYCHAIN_PATH"

# Import Apple intermediate certificates for a complete chain.
# Without these, Apple's notary rejects the signature as invalid.
CERT_TMP=$(mktemp -d)
curl -sL "https://www.apple.com/certificateauthority/DeveloperIDG2CA.cer" -o "$CERT_TMP/DeveloperIDG2CA.cer"
curl -sL "https://www.apple.com/appleca/AppleIncRootCertificate.cer" -o "$CERT_TMP/AppleIncRoot.cer"
security import "$CERT_TMP/DeveloperIDG2CA.cer" -k "$KEYCHAIN_PATH" -T /usr/bin/codesign || true
security import "$CERT_TMP/AppleIncRoot.cer" -k "$KEYCHAIN_PATH" -T /usr/bin/codesign || true
rm -rf "$CERT_TMP"
echo "    Imported Apple intermediate certificates"

# Allow codesign to access the keychain without prompting.
security set-key-partition-list -S apple-tool:,apple:,codesign: \
    -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"

# Add the temporary keychain to the search list (replace, don't append,
# to avoid picking up stale identities from other keychains).
security list-keychains -d user -s "$KEYCHAIN_PATH" $(security list-keychains -d user | tr -d '"')

# List all signing identities for diagnostics.
echo "    Available identities:"
security find-identity -v -p codesigning "$KEYCHAIN_PATH"

# Find the signing identity.
IDENTITY=$(security find-identity -v -p codesigning "$KEYCHAIN_PATH" \
    | grep "Developer ID Application" \
    | head -1 \
    | sed 's/.*"\(.*\)"/\1/')

if [ -z "$IDENTITY" ]; then
    echo "No 'Developer ID Application' identity found in certificate" >&2
    echo "The .p12 must contain a 'Developer ID Application' certificate." >&2
    echo "An 'Apple Developer' or 'Apple Development' certificate will not work." >&2
    exit 1
fi

echo "    Signing identity: $IDENTITY"

# ── 2. Sign ALL executables and libraries in the bundle ──────────
echo "==> Signing Slashpad.app"

ENTITLEMENTS="$REPO_ROOT/macos/entitlements.plist"

# Find and sign all Mach-O binaries, dylibs, and .so/.node files
# inside the bundle (inside-out: deepest first).
# This catches native node modules, bun, and anything else.
echo "    Signing nested binaries..."
find "$APP_DIR/Contents/Resources" -type f \( \
    -name "*.dylib" -o -name "*.so" -o -name "*.node" -o \
    -perm +111 \
\) | while read -r binary; do
    # Check if it's actually a Mach-O file (not a shell script or text file).
    if file "$binary" | grep -q "Mach-O"; then
        codesign --force --timestamp --options runtime \
            --entitlements "$ENTITLEMENTS" \
            --sign "$IDENTITY" \
            "$binary"
        echo "      Signed: ${binary#$APP_DIR/}"
    fi
done

# Sign the entire .app bundle with --deep to ensure consistent
# signatures throughout. The --deep flag signs the main executable
# and re-signs nested code with the same identity.
codesign --deep --force --timestamp --options runtime \
    --entitlements "$ENTITLEMENTS" \
    --sign "$IDENTITY" \
    "$APP_DIR"
echo "    Signed Slashpad.app (deep)"

# Verify the signature.
codesign --verify --deep --strict --verbose=2 "$APP_DIR"
echo "    Signature verified"

# Print signature details for the main binary (diagnostics).
echo "    Main binary signature details:"
codesign -dvv "$APP_DIR/Contents/MacOS/slashpad" 2>&1 | head -20

# ── 3. Notarize ─────────────────────────────────────────────────
echo "==> Notarizing Slashpad.app"

# Create a zip for notarization submission (notarytool requires a zip/dmg/pkg).
NOTARIZE_ZIP="$BUILD_DIR/Slashpad-notarize.zip"
(cd "$BUILD_DIR" && zip -qr "Slashpad-notarize.zip" Slashpad.app)

# Submit and capture the submission ID.
SUBMIT_OUTPUT=$(xcrun notarytool submit "$NOTARIZE_ZIP" \
    --apple-id "$APPLE_ID" \
    --password "$APPLE_ID_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" \
    --wait \
    --output-format json 2>&1) || true

echo "$SUBMIT_OUTPUT"

# Extract status and ID from JSON output.
NOTARIZE_STATUS=$(echo "$SUBMIT_OUTPUT" | python3 -c "import sys,json; print(json.load(sys.stdin).get('status','unknown'))" 2>/dev/null || echo "unknown")
SUBMISSION_ID=$(echo "$SUBMIT_OUTPUT" | python3 -c "import sys,json; print(json.load(sys.stdin).get('id',''))" 2>/dev/null || echo "")

if [ "$NOTARIZE_STATUS" != "Accepted" ]; then
    echo "    ERROR: Notarization failed with status: $NOTARIZE_STATUS"
    # Fetch the detailed log to see what Apple rejected.
    if [ -n "$SUBMISSION_ID" ]; then
        echo "    Fetching notarization log..."
        xcrun notarytool log "$SUBMISSION_ID" \
            --apple-id "$APPLE_ID" \
            --password "$APPLE_ID_PASSWORD" \
            --team-id "$APPLE_TEAM_ID" \
            developer_log.json 2>&1 || true
        cat developer_log.json 2>/dev/null || true
    fi
    exit 1
fi
echo "    Notarization accepted"

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
