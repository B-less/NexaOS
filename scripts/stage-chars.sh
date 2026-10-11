#!/bin/sh
# Stage the nexa-chars GTK emoji/character picker into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-chars" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-chars/nexa_chars"
cp -a "$base/chars/nexa_chars" "$rootfs/usr/local/lib/nexa-chars/nexa_chars"
find "$rootfs/usr/local/lib/nexa-chars/nexa_chars" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-chars:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_chars "$@"' \
    > "$rootfs/usr/local/bin/nexa-chars"
chmod +x "$rootfs/usr/local/bin/nexa-chars"

cat > "$rootfs/usr/share/applications/nexa-chars.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Nexa Characters
Comment=Pick and copy emoji and symbols
Exec=/usr/local/bin/nexa-chars
Icon=nexa-chars
Terminal=false
Categories=Utility;
DESKTOP_EOF

echo "nexa-chars staged into $rootfs"
