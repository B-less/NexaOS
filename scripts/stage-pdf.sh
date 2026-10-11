#!/bin/sh
# Stage the nexa-pdf GTK/Poppler PDF viewer into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-pdf" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-pdf/nexa_pdf"
cp -a "$base/pdf-viewer/nexa_pdf" "$rootfs/usr/local/lib/nexa-pdf/nexa_pdf"
find "$rootfs/usr/local/lib/nexa-pdf/nexa_pdf" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-pdf:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_pdf "$@"' \
    > "$rootfs/usr/local/bin/nexa-pdf"
chmod +x "$rootfs/usr/local/bin/nexa-pdf"

cat > "$rootfs/usr/share/applications/nexa-pdf.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Nexa PDF
Comment=View PDF documents
Exec=/usr/local/bin/nexa-pdf %f
Icon=nexa-pdf
Terminal=false
Categories=Office;Viewer;
MimeType=application/pdf;
DESKTOP_EOF

echo "nexa-pdf staged into $rootfs"
