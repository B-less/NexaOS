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
    '# WebKitGTK enables its bubblewrap-based sandbox by default and can refuse' \
    '# to launch a web process without it; this rootfs does not ship bubblewrap' \
    '# (a minimal, root-trusted, single-user system already outside that' \
    '# sandbox'"'"'s threat model, same tradeoff as the rest of this OS).' \
    '# bubblewrap + xdg-dbus-proxy are installed and the kernel supports user' \
    '# namespaces now, but the sandbox has not been verified against real web' \
    '# content yet, so it stays opt-in: run with NEXA_BROWSER_SANDBOX=1 to try it.' \
    '[ "${NEXA_BROWSER_SANDBOX:-0}" = "1" ] || export WEBKIT_DISABLE_SANDBOX=1' \
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
