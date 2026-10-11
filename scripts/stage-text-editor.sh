#!/bin/sh
# Stage the nexa-edit GTK/GtkSourceView text editor into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-edit" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-edit/nexa_edit"
cp -a "$base/text-editor/nexa_edit" "$rootfs/usr/local/lib/nexa-edit/nexa_edit"
find "$rootfs/usr/local/lib/nexa-edit/nexa_edit" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-edit:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_edit "$@"' \
    > "$rootfs/usr/local/bin/nexa-edit"
chmod +x "$rootfs/usr/local/bin/nexa-edit"

cat > "$rootfs/usr/share/applications/nexa-edit.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Editor
Comment=Edit text and code, with syntax highlighting
Exec=/usr/local/bin/nexa-edit %f
Icon=nexa-edit
Terminal=false
Categories=Utility;TextEditor;
MimeType=text/plain;application/x-shellscript;text/x-python;text/x-csrc;text/x-chdr;application/json;text/markdown;text/x-yaml;text/xml;text/css;application/toml;
DESKTOP_EOF

echo "nexa-edit staged into $rootfs"
