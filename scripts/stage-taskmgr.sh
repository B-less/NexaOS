#!/bin/sh
# Stage the nexa-taskmgr GTK process viewer into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-taskmgr" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-taskmgr/nexa_taskmgr"
cp -a "$base/taskmgr/nexa_taskmgr" "$rootfs/usr/local/lib/nexa-taskmgr/nexa_taskmgr"
find "$rootfs/usr/local/lib/nexa-taskmgr/nexa_taskmgr" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-taskmgr:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_taskmgr "$@"' \
    > "$rootfs/usr/local/bin/nexa-taskmgr"
chmod +x "$rootfs/usr/local/bin/nexa-taskmgr"

cat > "$rootfs/usr/share/applications/nexa-taskmgr.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Nexa Tasks
Comment=View and end running processes
Exec=/usr/local/bin/nexa-taskmgr
Icon=nexa-taskmgr
Terminal=false
Categories=System;Monitor;
NoDisplay=true
DESKTOP_EOF

echo "nexa-taskmgr staged into $rootfs"
