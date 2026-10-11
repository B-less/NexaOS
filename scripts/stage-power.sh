#!/bin/sh
# Stage the power menu (nexa-power: lock/log out/restart/shut down), its
# narrow privileged helper, and nexa-idle (swayidle auto-lock/screen-off).
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-power" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-power/nexa_power"
cp -a "$base/power/nexa_power" "$rootfs/usr/local/lib/nexa-power/nexa_power"
find "$rootfs/usr/local/lib/nexa-power/nexa_power" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-power:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_power "$@"' \
    > "$rootfs/usr/local/bin/nexa-power"
chmod +x "$rootfs/usr/local/bin/nexa-power"

printf '%s\n' \
    '#!/bin/sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-power:${PYTHONPATH:-}" python3 -m nexa_power.helper' \
    > "$rootfs/usr/local/bin/nexa-power-helper"
chmod +x "$rootfs/usr/local/bin/nexa-power-helper"

for script in nexa-idle nexa-lid nexa-battery-watch; do
    cp "$base/power/$script" "$rootfs/usr/local/bin/$script"
    chmod +x "$rootfs/usr/local/bin/$script"
done

if [ ! -f "$rootfs/etc/doas.conf" ]; then
    : > "$rootfs/etc/doas.conf"
fi
if ! grep -q nexa-power-helper "$rootfs/etc/doas.conf" 2>/dev/null; then
    echo 'permit nopass user cmd /usr/local/bin/nexa-power-helper' >> "$rootfs/etc/doas.conf"
fi
chown 0:0 "$rootfs/etc/doas.conf"
chmod 400 "$rootfs/etc/doas.conf"

cat > "$rootfs/usr/share/applications/nexa-power.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Power
Comment=Lock, log out, restart or shut down
Exec=/usr/local/bin/nexa-power
Icon=nexa-power
Terminal=false
Categories=System;
DESKTOP_EOF

echo "nexa-power staged into $rootfs"
