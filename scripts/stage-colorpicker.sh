#!/bin/sh
# Stage the nexa-colorpicker GTK screen color picker into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-colorpicker" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-colorpicker/nexa_colorpicker"
cp -a "$base/colorpicker/nexa_colorpicker" "$rootfs/usr/local/lib/nexa-colorpicker/nexa_colorpicker"
find "$rootfs/usr/local/lib/nexa-colorpicker/nexa_colorpicker" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-colorpicker:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_colorpicker "$@"' \
    > "$rootfs/usr/local/bin/nexa-colorpicker"
chmod +x "$rootfs/usr/local/bin/nexa-colorpicker"

cat > "$rootfs/usr/share/applications/nexa-colorpicker.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Nexa Color Picker
Comment=Sample a color from the screen
Exec=/usr/local/bin/nexa-colorpicker
Icon=nexa-colorpicker
Terminal=false
Categories=Utility;Graphics;
DESKTOP_EOF

echo "nexa-colorpicker staged into $rootfs"
