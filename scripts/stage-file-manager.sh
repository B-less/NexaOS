#!/bin/sh
# Stage the nexa-files GTK file manager into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-files" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-files/nexa_files"
cp -a "$base/file-manager/nexa_files" "$rootfs/usr/local/lib/nexa-files/nexa_files"
find "$rootfs/usr/local/lib/nexa-files/nexa_files" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-files:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_files "$@"' \
    > "$rootfs/usr/local/bin/nexa-files"
chmod +x "$rootfs/usr/local/bin/nexa-files"

cat > "$rootfs/usr/share/applications/nexa-files.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Files
Comment=Browse and manage files
Exec=/usr/local/bin/nexa-files
Icon=nexa-files
Terminal=false
Categories=System;Utility;
DESKTOP_EOF

echo "nexa-files staged into $rootfs"
