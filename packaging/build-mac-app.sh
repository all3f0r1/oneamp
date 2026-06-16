#!/usr/bin/env bash
# Build a universal macOS .app bundle and a .dmg disk image for OneAmp.
#
# Inputs:
#   $VERSION — workspace version (auto-read from Cargo.toml when unset)
#
# Outputs (under packaging/):
#   OneAmp.app/                        — universal .app bundle
#   OneAmp-<version>-mac-universal.dmg — UDZO-compressed disk image
#
# Requires macOS host tools: cargo + rustup targets, lipo, sips,
# iconutil, hdiutil, plutil. All preinstalled on `macos-latest`
# runners.
#
# Unsigned / unnotarised: Gatekeeper will require right-click → Open
# on first launch. Signing is Phase 3, gated on an Apple Developer
# account.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

if [[ -z "${VERSION:-}" ]]; then
    VERSION="$(awk '/^\[workspace\.package\]/{f=1; next} f && /^version[[:space:]]*=/{gsub(/[" ]/,"",$3); print $3; exit}' "$PROJECT_ROOT/Cargo.toml")"
fi
if [[ -z "$VERSION" ]]; then
    echo "ERROR: could not read workspace version from Cargo.toml" >&2
    exit 1
fi

echo "Building OneAmp ${VERSION} macOS .app + .dmg..."

cd "$PROJECT_ROOT"

# 1. Build the binary for both architectures and fuse via lipo. The
#    macos-latest runner is arm64 by default; `cargo build` for the
#    x86_64 target cross-compiles via the toolchain we asked rustup
#    to install in the workflow.
echo "==> Building x86_64..."
cargo build --release --target x86_64-apple-darwin -p oneamp-desktop
echo "==> Building aarch64..."
cargo build --release --target aarch64-apple-darwin -p oneamp-desktop

UNIVERSAL_DIR="$PROJECT_ROOT/target/universal-apple-darwin/release"
mkdir -p "$UNIVERSAL_DIR"
echo "==> lipo merge..."
lipo -create \
    "$PROJECT_ROOT/target/x86_64-apple-darwin/release/oneamp" \
    "$PROJECT_ROOT/target/aarch64-apple-darwin/release/oneamp" \
    -output "$UNIVERSAL_DIR/oneamp"
strip "$UNIVERSAL_DIR/oneamp" || true

# 2. Generate OneAmp.icns from the existing icon set. iconutil wants an
#    `.iconset` directory with specifically named PNGs. We map the
#    closest source size from packaging/icons/ into each slot — exact
#    matches where possible, sips upscale for the few that have no
#    direct source.
echo "==> Building OneAmp.icns..."
ICONSET="$SCRIPT_DIR/build/OneAmp.iconset"
rm -rf "$ICONSET"
mkdir -p "$ICONSET"
cp "$SCRIPT_DIR/icons/oneamp-16.png"  "$ICONSET/icon_16x16.png"
cp "$SCRIPT_DIR/icons/oneamp-32.png"  "$ICONSET/icon_16x16@2x.png"
cp "$SCRIPT_DIR/icons/oneamp-32.png"  "$ICONSET/icon_32x32.png"
cp "$SCRIPT_DIR/icons/oneamp-64.png"  "$ICONSET/icon_32x32@2x.png"
cp "$SCRIPT_DIR/icons/oneamp-128.png" "$ICONSET/icon_128x128.png"
cp "$SCRIPT_DIR/icons/oneamp-256.png" "$ICONSET/icon_128x128@2x.png"
cp "$SCRIPT_DIR/icons/oneamp-256.png" "$ICONSET/icon_256x256.png"
cp "$SCRIPT_DIR/icons/oneamp-512.png" "$ICONSET/icon_256x256@2x.png"
cp "$SCRIPT_DIR/icons/oneamp-512.png" "$ICONSET/icon_512x512.png"
# 1024 PNG would be ideal here; we upscale the 512 via sips. macOS
# users on a Studio Display see the result, no one else.
sips -z 1024 1024 "$SCRIPT_DIR/icons/oneamp-512.png" \
    --out "$ICONSET/icon_512x512@2x.png" >/dev/null

iconutil -c icns "$ICONSET" -o "$SCRIPT_DIR/build/OneAmp.icns"

# 3. Assemble the .app bundle.
echo "==> Assembling OneAmp.app..."
APP_DIR="$SCRIPT_DIR/build/OneAmp.app"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"
cp "$UNIVERSAL_DIR/oneamp" "$APP_DIR/Contents/MacOS/oneamp"
chmod 755 "$APP_DIR/Contents/MacOS/oneamp"
cp "$SCRIPT_DIR/build/OneAmp.icns" "$APP_DIR/Contents/Resources/OneAmp.icns"
sed "s/@VERSION@/${VERSION}/g" \
    "$SCRIPT_DIR/macos/Info.plist.in" \
    > "$APP_DIR/Contents/Info.plist"
# Convert to binary plist (smaller, what Apple recommends for shipped
# bundles). plutil rejects invalid XML which doubles as validation.
plutil -convert binary1 "$APP_DIR/Contents/Info.plist"

# 4. Package as a UDZO-compressed read-only DMG.
echo "==> Creating DMG..."
DMG_PATH="$SCRIPT_DIR/OneAmp-${VERSION}-mac-universal.dmg"
rm -f "$DMG_PATH"
hdiutil create \
    -volname "OneAmp ${VERSION}" \
    -srcfolder "$APP_DIR" \
    -ov \
    -format UDZO \
    "$DMG_PATH"

echo ""
echo "✓ Built OneAmp.app + DMG"
echo "  App:  $APP_DIR"
echo "  DMG:  $DMG_PATH"
echo "  Size: $(du -h "$DMG_PATH" | cut -f1)"
