#!/bin/sh
# Stage the nexa-sysmon GTK system monitor into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-sysmon" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-sysmon/nexa_sysmon"
cp -a "$base/sysmon/nexa_sysmon" "$rootfs/usr/local/lib/nexa-sysmon/nexa_sysmon"
find "$rootfs/usr/local/lib/nexa-sysmon/nexa_sysmon" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-sysmon:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_sysmon "$@"' \
    > "$rootfs/usr/local/bin/nexa-sysmon"
chmod +x "$rootfs/usr/local/bin/nexa-sysmon"

cat > "$rootfs/usr/share/applications/nexa-sysmon.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Nexa System Monitor
Comment=CPU, memory, and disk usage
Exec=/usr/local/bin/nexa-sysmon
Icon=nexa-sysmon
Terminal=false
Categories=System;Monitor;
NoDisplay=true
DESKTOP_EOF

echo "nexa-sysmon staged into $rootfs"
