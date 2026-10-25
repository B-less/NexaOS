#!/bin/sh
# Stage nexa-lock (starts nexa-locker, swaylock as fallback) into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

cp "$base/lockscreen/nexa-lock" "$rootfs/usr/local/bin/nexa-lock"
chmod +x "$rootfs/usr/local/bin/nexa-lock"
# PAM service for nexa-locker (the program itself is staged with the
# native tools, from build/native).
mkdir -p "$rootfs/etc/pam.d"
install -m 0644 "$base/lockscreen/nexa-lock.pam" "$rootfs/etc/pam.d/nexa-lock"

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
