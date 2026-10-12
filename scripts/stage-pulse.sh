#!/bin/sh
# Stage Nexa Pulse (task manager + system monitor) into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-pulse" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-pulse/nexa_pulse"
cp -a "$base/pulse/nexa_pulse" "$rootfs/usr/local/lib/nexa-pulse/nexa_pulse"
find "$rootfs/usr/local/lib/nexa-pulse/nexa_pulse" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-pulse:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_pulse "$@"' \
    > "$rootfs/usr/local/bin/nexa-pulse"
chmod +x "$rootfs/usr/local/bin/nexa-pulse"

cat > "$rootfs/usr/share/applications/nexa-pulse.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Nexa Pulse
Comment=See and end running apps; live CPU, memory, disk and network
Exec=/usr/local/bin/nexa-pulse
Icon=nexa-pulse
Terminal=false
Categories=System;Monitor;
Keywords=task manager;system monitor;processes;performance;
DESKTOP_EOF

echo "nexa-pulse staged into $rootfs"
