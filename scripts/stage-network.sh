#!/bin/sh
# Stage the nexa-network GTK network settings app into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-network" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-network/nexa_network"
cp -a "$base/network-settings/nexa_network" "$rootfs/usr/local/lib/nexa-network/nexa_network"
find "$rootfs/usr/local/lib/nexa-network/nexa_network" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-network:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_network "$@"' \
    > "$rootfs/usr/local/bin/nexa-network"
chmod +x "$rootfs/usr/local/bin/nexa-network"

cat > "$rootfs/usr/share/applications/nexa-network.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Network
Comment=Connect to WiFi networks
Exec=/usr/local/bin/nexa-network
Icon=nexa-network
Terminal=false
Categories=Network;Settings;
DESKTOP_EOF

echo "nexa-network staged into $rootfs"
