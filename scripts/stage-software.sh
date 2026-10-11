#!/bin/sh
# Stage the nexa-software GTK package manager (apk front-end) into the rootfs.
#
# Appends to /etc/doas.conf rather than overwriting it, same as
# stage-settings.sh, so every privileged helper staged so far stays allowed
# regardless of staging order.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-software" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-software/nexa_software"
cp -a "$base/software-center/nexa_software" "$rootfs/usr/local/lib/nexa-software/nexa_software"
find "$rootfs/usr/local/lib/nexa-software/nexa_software" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-software:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_software "$@"' \
    > "$rootfs/usr/local/bin/nexa-software"
chmod +x "$rootfs/usr/local/bin/nexa-software"

printf '%s\n' \
    '#!/bin/sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-software:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_software.control_helper' \
    > "$rootfs/usr/local/bin/nexa-software-helper"
chmod +x "$rootfs/usr/local/bin/nexa-software-helper"

if [ ! -f "$rootfs/etc/doas.conf" ]; then
    : > "$rootfs/etc/doas.conf"
fi
if ! grep -q nexa-software-helper "$rootfs/etc/doas.conf" 2>/dev/null; then
    echo 'permit nopass user cmd /usr/local/bin/nexa-software-helper' >> "$rootfs/etc/doas.conf"
fi
chown 0:0 "$rootfs/etc/doas.conf"
chmod 400 "$rootfs/etc/doas.conf"

cat > "$rootfs/usr/share/applications/nexa-software.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Nexa Software
Comment=Install and remove software
Exec=/usr/local/bin/nexa-software
Icon=nexa-software
Terminal=false
Categories=System;PackageManager;
DESKTOP_EOF

echo "nexa-software staged into $rootfs"
