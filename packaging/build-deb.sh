#!/bin/bash
set -e

# OneAmp Debian Package Builder
# This script creates a .deb package for OneAmp

PACKAGE_NAME="oneamp"
VERSION="0.5.0"
ARCH="amd64"
MAINTAINER="OneAmp Team <oneamp@example.com>"
DESCRIPTION="Modern audio player inspired by Winamp"

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
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
cp "$SCRIPT_DIR/oneamp.desktop" "$DEB_DIR/usr/share/applications/"

# Copy icons
echo "Copying icons..."
cp "$SCRIPT_DIR/icons/oneamp-16.png" "$DEB_DIR/usr/share/icons/hicolor/16x16/apps/oneamp.png"
cp "$SCRIPT_DIR/icons/oneamp-32.png" "$DEB_DIR/usr/share/icons/hicolor/32x32/apps/oneamp.png"
cp "$SCRIPT_DIR/icons/oneamp-48.png" "$DEB_DIR/usr/share/icons/hicolor/48x48/apps/oneamp.png"
cp "$SCRIPT_DIR/icons/oneamp-64.png" "$DEB_DIR/usr/share/icons/hicolor/64x64/apps/oneamp.png"
cp "$SCRIPT_DIR/icons/oneamp-128.png" "$DEB_DIR/usr/share/icons/hicolor/128x128/apps/oneamp.png"
cp "$SCRIPT_DIR/icons/oneamp-256.png" "$DEB_DIR/usr/share/icons/hicolor/256x256/apps/oneamp.png"
cp "$SCRIPT_DIR/icons/oneamp-512.png" "$DEB_DIR/usr/share/icons/hicolor/512x512/apps/oneamp.png"

# Copy documentation
echo "Copying documentation..."
cp "$PROJECT_ROOT/README.md" "$DEB_DIR/usr/share/doc/$PACKAGE_NAME/"
cp "$PROJECT_ROOT/LICENSE" "$DEB_DIR/usr/share/doc/$PACKAGE_NAME/"

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
Depends: libasound2 (>= 1.0.16), libc6 (>= 2.34)
Maintainer: $MAINTAINER
Description: $DESCRIPTION
 OneAmp is a modern audio player for Linux inspired by Winamp.
 It supports MP3 and FLAC playback, features a 10-band equalizer,
 playlist management, and a clean modern interface.
 .
 Features:
  - MP3 and FLAC playback
  - 10-band graphic equalizer
  - Playlist management
  - Modern dark theme
  - Low resource usage
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
