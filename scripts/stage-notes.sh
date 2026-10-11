#!/bin/sh
# Stage the nexa-notes GTK sticky notes app into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-notes" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-notes/nexa_notes"
cp -a "$base/notes/nexa_notes" "$rootfs/usr/local/lib/nexa-notes/nexa_notes"
find "$rootfs/usr/local/lib/nexa-notes/nexa_notes" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-notes:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_notes "$@"' \
    > "$rootfs/usr/local/bin/nexa-notes"
chmod +x "$rootfs/usr/local/bin/nexa-notes"

cat > "$rootfs/usr/share/applications/nexa-notes.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Nexa Notes
Comment=Sticky notes
Exec=/usr/local/bin/nexa-notes
Icon=nexa-notes
Terminal=false
Categories=Utility;Office;
DESKTOP_EOF

echo "nexa-notes staged into $rootfs"
