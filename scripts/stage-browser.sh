#!/bin/sh
# Stage the nexa-browser GTK/WebKit2 web browser into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-browser" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-browser/nexa_browser"
cp -a "$base/browser/nexa_browser" "$rootfs/usr/local/lib/nexa-browser/nexa_browser"
find "$rootfs/usr/local/lib/nexa-browser/nexa_browser" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    '# Web content runs in WebKit'"'"'s bubblewrap sandbox (verified on Alpine' \
    '# 3.24: pages and JavaScript work with it on). NEXA_BROWSER_SANDBOX=0' \
    '# turns it off for troubleshooting only.' \
    '[ "${NEXA_BROWSER_SANDBOX:-1}" = "0" ] && export WEBKIT_DISABLE_SANDBOX=1' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-browser:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_browser "$@"' \
    > "$rootfs/usr/local/bin/nexa-browser"
chmod +x "$rootfs/usr/local/bin/nexa-browser"

cat > "$rootfs/usr/share/applications/nexa-browser.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Nexa Browser
Comment=Browse the web
Exec=/usr/local/bin/nexa-browser
Icon=nexa-browser
Terminal=false
Categories=Network;WebBrowser;
DESKTOP_EOF

echo "nexa-browser staged into $rootfs"
