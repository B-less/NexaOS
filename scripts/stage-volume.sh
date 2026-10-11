#!/bin/sh
# Stage the nexa-volume GTK volume mixer into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-volume" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-volume/nexa_volume"
cp -a "$base/volume-mixer/nexa_volume" "$rootfs/usr/local/lib/nexa-volume/nexa_volume"
find "$rootfs/usr/local/lib/nexa-volume/nexa_volume" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-volume:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_volume "$@"' \
    > "$rootfs/usr/local/bin/nexa-volume"
chmod +x "$rootfs/usr/local/bin/nexa-volume"

cat > "$rootfs/usr/share/applications/nexa-volume.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Volume
Comment=Adjust system output volume
Exec=/usr/local/bin/nexa-volume
Icon=nexa-volume
Terminal=false
Categories=AudioVideo;Settings;
DESKTOP_EOF

echo "nexa-volume staged into $rootfs"
