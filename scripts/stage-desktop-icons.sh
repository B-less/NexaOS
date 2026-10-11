#!/bin/sh
# Stage nexa-desktop-icons: a GTK Layer Shell surface, anchored to the
# BOTTOM layer (above the wallpaper's BACKGROUND layer, below normal
# windows), that renders clickable icons for every app in
# /usr/share/applications. Sway is a tiling compositor with no built-in
# concept of desktop icons — this is what actually puts them there.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-desktop-icons" "$rootfs/usr/local/bin"

rm -rf "$rootfs/usr/local/lib/nexa-desktop-icons/nexa_desktop_icons"
cp -a "$base/desktop-icons/nexa_desktop_icons" "$rootfs/usr/local/lib/nexa-desktop-icons/nexa_desktop_icons"
find "$rootfs/usr/local/lib/nexa-desktop-icons/nexa_desktop_icons" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-desktop-icons:${PYTHONPATH:-}" python3 -m nexa_desktop_icons "$@"' \
    > "$rootfs/usr/local/bin/nexa-desktop-icons"
chmod +x "$rootfs/usr/local/bin/nexa-desktop-icons"

echo "nexa-desktop-icons staged into $rootfs"
