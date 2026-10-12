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

# nexadark: a private browser window that goes through Tor. Tor runs as the
# user on 127.0.0.1 only, started on demand (see nexa_browser/tor.py).
cat > "$rootfs/usr/local/bin/nexadark" << 'NEXADARK_EOF'
#!/bin/sh
case "${1:-}" in
    status|stop)
        exec env PYTHONPATH="/usr/local/lib/nexa-browser:${PYTHONPATH:-}" python3 -m nexa_browser.tor "$1" ;;
    -h|--help)
        echo "usage: nexadark [URL]   open a private browser window that goes through Tor"
        echo "       nexadark status  is Tor running / connected"
        echo "       nexadark stop    stop Tor"
        exit 0 ;;
esac
echo "Opening a Tor window. Nothing from it is saved; it hides your IP address,"
echo "but it is not Tor Browser (sites can still fingerprint it)."
exec /usr/local/bin/nexa-browser --tor "$@" >/dev/null 2>&1
NEXADARK_EOF
chmod +x "$rootfs/usr/local/bin/nexadark"

cat > "$rootfs/usr/share/applications/nexa-browser.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Nexa Browser
Comment=Browse the web
Exec=/usr/local/bin/nexa-browser
Icon=nexa-browser
Terminal=false
Categories=Network;WebBrowser;
Actions=private;tor;

[Desktop Action private]
Name=New private window
Exec=/usr/local/bin/nexa-browser --private

[Desktop Action tor]
Name=New Tor window
Exec=/usr/local/bin/nexadark
DESKTOP_EOF

cat > "$rootfs/usr/share/applications/nexa-browser-tor.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Tor Window
Comment=Private browsing through the Tor network
Exec=/usr/local/bin/nexadark
Icon=nexa-browser
Terminal=false
Categories=Network;WebBrowser;
Keywords=tor;private;anonymous;nexadark;onion;
DESKTOP_EOF

echo "nexa-browser staged into $rootfs"
