#!/bin/sh
# Stage the nexa-screenshot capture tool (grim/slurp wrapper) into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

cp "$base/screenshot/nexa-screenshot" "$rootfs/usr/local/bin/nexa-screenshot"
chmod +x "$rootfs/usr/local/bin/nexa-screenshot"

cat > "$rootfs/usr/share/applications/nexa-screenshot.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Screenshot
Comment=Capture the screen
Exec=/usr/local/bin/nexa-screenshot
Icon=nexa-screenshot
Terminal=false
Categories=Utility;Graphics;
DESKTOP_EOF

echo "nexa-screenshot staged into $rootfs"
