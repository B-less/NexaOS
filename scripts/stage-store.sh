#!/bin/sh
# Stage the Nexa Store into the rootfs: the window (store/nexa_store, a thin
# GTK front end over the Rust nexa-store engine and the system broker), its
# built-in signed catalogue of NexaOS's own apps (build/store-repo, made by
# scripts/build-apps.sh) and /etc/nexa/store.conf naming the catalogues.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-store" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications" \
    "$rootfs/usr/share/nexa-store" "$rootfs/etc/nexa"

rm -rf "$rootfs/usr/local/lib/nexa-store/nexa_store"
cp -a "$base/store/nexa_store" "$rootfs/usr/local/lib/nexa-store/nexa_store"
find "$rootfs/usr/local/lib/nexa-store/nexa_store" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-store:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_store "$@"' \
    > "$rootfs/usr/local/bin/nexa-store-app"
chmod +x "$rootfs/usr/local/bin/nexa-store-app"

rm -rf "$rootfs/usr/share/nexa-store/repo"
if [ -f "$base/build/store-repo/index.json" ]; then
    cp -a "$base/build/store-repo" "$rootfs/usr/share/nexa-store/repo"
    chmod -R a+rX "$rootfs/usr/share/nexa-store/repo"
else
    echo "warning: no build/store-repo (run make nexa-apps): the Store starts empty"
fi

# The catalogues the Store reads: the built-in one (works offline) and the
# online one (B-less/nexa-apps on GitHub Pages, published with
# `make publish-store`: new apps and updates without a new ISO). Both are
# signed with the NexaOS release key. More can be added (HTTPS or local
# folders; unsigned ones are shown as not verified).
cat > "$rootfs/etc/nexa/store.conf" << 'CONF_EOF'
{"catalogues": [
  {"name": "NexaOS", "url": "file:///usr/share/nexa-store/repo"},
  {"name": "NexaOS online", "url": "https://b-less.github.io/nexa-apps"}
]}
CONF_EOF
chmod 0644 "$rootfs/etc/nexa/store.conf"

cat > "$rootfs/usr/share/applications/nexa-store.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Nexa Store
Comment=Find, install, update and remove apps
Exec=/usr/local/bin/nexa-store-app %U
Icon=nexa-store
Terminal=false
Categories=System;PackageManager;
Keywords=apps;install;software;store;update;
DESKTOP_EOF

echo "nexa-store staged into $rootfs"
