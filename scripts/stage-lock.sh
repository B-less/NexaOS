#!/bin/sh
# Stage nexa-lock (a themed swaylock wrapper) into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

cp "$base/lockscreen/nexa-lock" "$rootfs/usr/local/bin/nexa-lock"
chmod +x "$rootfs/usr/local/bin/nexa-lock"

cat > "$rootfs/usr/share/applications/nexa-lock.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Lock Screen
Comment=Lock the current session
Exec=/usr/local/bin/nexa-lock
Icon=nexa-lock
Terminal=false
Categories=System;
DESKTOP_EOF

echo "nexa-lock staged into $rootfs"
