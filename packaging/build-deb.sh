#!/bin/bash
set -e

# OneAmp Debian Package Builder
# This script creates a .deb package for OneAmp

PACKAGE_NAME="oneamp"
APP_ID="io.github.all3f0r1.OneAmp"
ARCH="amd64"
MAINTAINER="OneAmp Team <oneamp@example.com>"
DESCRIPTION="Pixel-perfect Winamp 2.x in Rust — plays your music, looks good doing it"

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Read version from workspace Cargo.toml so the .deb stays in sync with crate
# releases. Matches the first `version = "x.y.z"` line under [workspace.package].
VERSION="$(awk '/^\[workspace\.package\]/{f=1; next} f && /^version[[:space:]]*=/{gsub(/[" ]/,"",$3); print $3; exit}' "$PROJECT_ROOT/Cargo.toml")"
if [ -z "$VERSION" ]; then
    echo "ERROR: could not read workspace version from Cargo.toml" >&2
    exit 1
fi

BUILD_DIR="$SCRIPT_DIR/build"
DEB_DIR="$BUILD_DIR/${PACKAGE_NAME}_${VERSION}_${ARCH}"

echo "Building OneAmp ${VERSION} Debian package..."

# Clean previous build
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"

# Create package directory structure
mkdir -p "$DEB_DIR/DEBIAN"
mkdir -p "$DEB_DIR/usr/bin"
mkdir -p "$DEB_DIR/usr/share/applications"
mkdir -p "$DEB_DIR/usr/share/icons/hicolor/16x16/apps"
mkdir -p "$DEB_DIR/usr/share/icons/hicolor/32x32/apps"
mkdir -p "$DEB_DIR/usr/share/icons/hicolor/48x48/apps"
mkdir -p "$DEB_DIR/usr/share/icons/hicolor/64x64/apps"
mkdir -p "$DEB_DIR/usr/share/icons/hicolor/128x128/apps"
mkdir -p "$DEB_DIR/usr/share/icons/hicolor/256x256/apps"
mkdir -p "$DEB_DIR/usr/share/icons/hicolor/512x512/apps"
mkdir -p "$DEB_DIR/usr/share/metainfo"
mkdir -p "$DEB_DIR/usr/share/doc/$PACKAGE_NAME"

# Build the release binary
echo "Building release binary..."
cd "$PROJECT_ROOT"
cargo build --release -p oneamp-desktop

# Copy binary
echo "Copying binary..."
cp "$PROJECT_ROOT/target/release/oneamp" "$DEB_DIR/usr/bin/"
strip "$DEB_DIR/usr/bin/oneamp"

# Copy desktop file
echo "Copying desktop file..."
cp "$SCRIPT_DIR/${APP_ID}.desktop" "$DEB_DIR/usr/share/applications/"

# Copy AppStream metainfo
echo "Copying AppStream metainfo..."
cp "$SCRIPT_DIR/${APP_ID}.metainfo.xml" "$DEB_DIR/usr/share/metainfo/"

# Copy icons (named after the AppStream/desktop ID so Icon= resolves)
echo "Copying icons..."
cp "$SCRIPT_DIR/icons/oneamp-16.png" "$DEB_DIR/usr/share/icons/hicolor/16x16/apps/${APP_ID}.png"
cp "$SCRIPT_DIR/icons/oneamp-32.png" "$DEB_DIR/usr/share/icons/hicolor/32x32/apps/${APP_ID}.png"
cp "$SCRIPT_DIR/icons/oneamp-48.png" "$DEB_DIR/usr/share/icons/hicolor/48x48/apps/${APP_ID}.png"
cp "$SCRIPT_DIR/icons/oneamp-64.png" "$DEB_DIR/usr/share/icons/hicolor/64x64/apps/${APP_ID}.png"
cp "$SCRIPT_DIR/icons/oneamp-128.png" "$DEB_DIR/usr/share/icons/hicolor/128x128/apps/${APP_ID}.png"
cp "$SCRIPT_DIR/icons/oneamp-256.png" "$DEB_DIR/usr/share/icons/hicolor/256x256/apps/${APP_ID}.png"
cp "$SCRIPT_DIR/icons/oneamp-512.png" "$DEB_DIR/usr/share/icons/hicolor/512x512/apps/${APP_ID}.png"

# Copy documentation
echo "Copying documentation..."
cp "$PROJECT_ROOT/README.md" "$DEB_DIR/usr/share/doc/$PACKAGE_NAME/"
cp "$PROJECT_ROOT/LICENSE-MIT" "$DEB_DIR/usr/share/doc/$PACKAGE_NAME/"
cp "$PROJECT_ROOT/LICENSE-APACHE" "$DEB_DIR/usr/share/doc/$PACKAGE_NAME/"

# Get installed size
INSTALLED_SIZE=$(du -sk "$DEB_DIR" | cut -f1)

# Create control file
echo "Creating control file..."
cat > "$DEB_DIR/DEBIAN/control" << EOF
Package: $PACKAGE_NAME
Version: $VERSION
Section: sound
Priority: optional
Architecture: $ARCH
Installed-Size: $INSTALLED_SIZE
Depends: libasound2 (>= 1.0.16), libc6 (>= 2.34), libxkbcommon0, libwayland-client0, libegl1, libgl1, libdbus-1-3
Recommends: pipewire-pulse | pulseaudio
Maintainer: $MAINTAINER
Description: $DESCRIPTION
 OneAmp is a native audio player in Rust, faithful to Winamp 2.x —
 your .wsz skins render pixel-perfect, your local library plays,
 internet radio streams, and the whole thing stays out of your way.
 No media library, no CD ripper, no podcast subscription manager,
 no "smart" anything. It's a music player. It plays music.
 .
 Features:
  - Decodes MP3, FLAC, OGG Vorbis, WAV, AAC, M4A/MP4, ALAC (Symphonia)
  - Native pixel-perfect rendering of Winamp 2.x .wsz skins, hot-swappable
  - 10-band RBJ-biquad equalizer (ISO 266 octave centres, +/-20 dB,
    constant-Q with coefficient ramping, .eqf preset import/export)
  - Per-preset auto-preamp so heavy presets don't clip the brickwall limiter
  - Internet-radio + podcast streaming over HTTP/HTTPS (Ctrl+L),
    live ICY now-playing metadata bridged to the playlist row
  - Built-in tag editor on the playlist right-click menu (ID3v1/v2,
    Vorbis, MP4 atoms, RIFF, APE) - preserves cover art and ReplayGain
  - Customisable playlist row format with {artist}/{title}/{album}/
    {tracknumber}/{year}/{duration}/{filename} tokens
  - Gapless transitions, equal-power crossfade, ReplayGain track gain,
    Fletcher-Munson loudness compensation, constant-power stereo balance
  - Spectrum analyzer, oscilloscope, peak/RMS meter visualizers
  - M3U / PLS playlist load/save, drag-and-drop ingest (files + folders)
  - MPRIS2 media bus, multimedia keys, track-change notifications,
    sleep timer, always-on-top
EOF

# Create postinst script
cat > "$DEB_DIR/DEBIAN/postinst" << 'EOF'
#!/bin/sh
set -e

# Update icon cache
if [ -x /usr/bin/gtk-update-icon-cache ]; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
fi

# Update desktop database
if [ -x /usr/bin/update-desktop-database ]; then
    update-desktop-database -q /usr/share/applications || true
fi

# Refresh AppStream metadata cache
if [ -x /usr/bin/appstreamcli ]; then
    appstreamcli refresh-cache --force >/dev/null 2>&1 || true
fi

exit 0
EOF

chmod 755 "$DEB_DIR/DEBIAN/postinst"

# Create postrm script
cat > "$DEB_DIR/DEBIAN/postrm" << 'EOF'
#!/bin/sh
set -e

# Update icon cache
if [ -x /usr/bin/gtk-update-icon-cache ]; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
fi

# Update desktop database
if [ -x /usr/bin/update-desktop-database ]; then
    update-desktop-database -q /usr/share/applications || true
fi

# Refresh AppStream metadata cache
if [ -x /usr/bin/appstreamcli ]; then
    appstreamcli refresh-cache --force >/dev/null 2>&1 || true
fi

exit 0
EOF

chmod 755 "$DEB_DIR/DEBIAN/postrm"

# Build the package
echo "Building .deb package..."
dpkg-deb --build "$DEB_DIR"

# Move to packaging directory
mv "$BUILD_DIR/${PACKAGE_NAME}_${VERSION}_${ARCH}.deb" "$SCRIPT_DIR/"

echo ""
echo "✓ Package built successfully!"
echo "  Location: $SCRIPT_DIR/${PACKAGE_NAME}_${VERSION}_${ARCH}.deb"
echo "  Size: $(du -h "$SCRIPT_DIR/${PACKAGE_NAME}_${VERSION}_${ARCH}.deb" | cut -f1)"
echo ""
echo "To install:"
echo "  sudo dpkg -i $SCRIPT_DIR/${PACKAGE_NAME}_${VERSION}_${ARCH}.deb"
echo ""
