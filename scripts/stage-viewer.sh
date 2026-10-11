#!/bin/sh
# Stage the nexa-viewer GTK image viewer into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-viewer" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-viewer/nexa_viewer"
cp -a "$base/image-viewer/nexa_viewer" "$rootfs/usr/local/lib/nexa-viewer/nexa_viewer"
find "$rootfs/usr/local/lib/nexa-viewer/nexa_viewer" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-viewer:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_viewer "$@"' \
    > "$rootfs/usr/local/bin/nexa-viewer"
chmod +x "$rootfs/usr/local/bin/nexa-viewer"

cat > "$rootfs/usr/share/applications/nexa-viewer.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Nexa Viewer
Comment=View images
Exec=/usr/local/bin/nexa-viewer %f
Icon=nexa-viewer
Terminal=false
Categories=Graphics;Viewer;
MimeType=image/png;image/jpeg;image/gif;image/bmp;image/svg+xml;image/webp;
DESKTOP_EOF

echo "nexa-viewer staged into $rootfs"
