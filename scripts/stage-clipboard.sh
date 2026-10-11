#!/bin/sh
# Stage the nexa-clipboard GTK clipboard history viewer into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-clipboard" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-clipboard/nexa_clipboard"
cp -a "$base/clipboard/nexa_clipboard" "$rootfs/usr/local/lib/nexa-clipboard/nexa_clipboard"
find "$rootfs/usr/local/lib/nexa-clipboard/nexa_clipboard" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-clipboard:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_clipboard "$@"' \
    > "$rootfs/usr/local/bin/nexa-clipboard"
chmod +x "$rootfs/usr/local/bin/nexa-clipboard"

cat > "$rootfs/usr/share/applications/nexa-clipboard.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Nexa Clipboard
Comment=Clipboard history
Exec=/usr/local/bin/nexa-clipboard
Icon=nexa-clipboard
Terminal=false
Categories=Utility;
DESKTOP_EOF

echo "nexa-clipboard staged into $rootfs"
