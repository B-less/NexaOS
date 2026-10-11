#!/bin/sh
# Stage the nexa-archive GTK archive manager (tar/zip via subprocess) into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-archive" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-archive/nexa_archive"
cp -a "$base/archive/nexa_archive" "$rootfs/usr/local/lib/nexa-archive/nexa_archive"
find "$rootfs/usr/local/lib/nexa-archive/nexa_archive" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-archive:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_archive "$@"' \
    > "$rootfs/usr/local/bin/nexa-archive"
chmod +x "$rootfs/usr/local/bin/nexa-archive"

cat > "$rootfs/usr/share/applications/nexa-archive.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Nexa Archive
Comment=Browse and extract archives
Exec=/usr/local/bin/nexa-archive %f
Icon=nexa-archive
Terminal=false
Categories=Utility;Archiving;
MimeType=application/x-tar;application/gzip;application/zip;application/x-bzip2;application/x-xz;application/x-compressed-tar;application/x-7z-compressed;
DESKTOP_EOF

echo "nexa-archive staged into $rootfs"
