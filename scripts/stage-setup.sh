#!/bin/sh
# Stage nexa-setup (the OS installer: partitions a real disk, installs GRUB
# for both BIOS and UEFI, clones the running system onto it) and its
# privileged helper. Not named nexa-install -- that's the .nexa package
# installer (see stage-installer.sh); this one wipes a physical disk.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-setup" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-setup/nexa_setup"
cp -a "$base/installer-os/nexa_setup" "$rootfs/usr/local/lib/nexa-setup/nexa_setup"
find "$rootfs/usr/local/lib/nexa-setup/nexa_setup" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-setup:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_setup "$@"' \
    > "$rootfs/usr/local/bin/nexa-setup"
chmod +x "$rootfs/usr/local/bin/nexa-setup"

printf '%s\n' \
    '#!/bin/sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-setup:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_setup.helper' \
    > "$rootfs/usr/local/bin/nexa-setup-helper"
chmod +x "$rootfs/usr/local/bin/nexa-setup-helper"

if [ ! -f "$rootfs/etc/doas.conf" ]; then
    : > "$rootfs/etc/doas.conf"
fi
if ! grep -q nexa-setup-helper "$rootfs/etc/doas.conf" 2>/dev/null; then
    echo 'permit nopass user cmd /usr/local/bin/nexa-setup-helper' >> "$rootfs/etc/doas.conf"
fi
chown 0:0 "$rootfs/etc/doas.conf"
chmod 400 "$rootfs/etc/doas.conf"

cat > "$rootfs/usr/share/applications/nexa-setup.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Install NexaOS
Comment=Install NexaOS onto a disk
Exec=/usr/local/bin/nexa-setup
Icon=nexa-setup
Terminal=false
Categories=System;
DESKTOP_EOF

echo "nexa-setup staged into $rootfs"
