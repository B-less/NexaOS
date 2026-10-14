#!/bin/sh
# Stage USB automount: the udev rules that mount removable drives under
# /media on plug-in and clean up on unplug (nexa-brokerd drive-added /
# drive-removed); the Files app's Devices menu asks nexa-brokerd too.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/bin" "$rootfs/etc/udev/rules.d" "$rootfs/media"
chown 0:0 "$rootfs/media"
chmod 755 "$rootfs/media"

rm -rf "$rootfs/usr/local/lib/nexa-automount"

# Drives are mounted by nexa-brokerd: the Files app asks it, udev runs it directly.
rm -f "$rootfs/usr/local/bin/nexa-mount-helper"
if [ -f "$rootfs/etc/doas.conf" ]; then
    sed -i '/nexa-mount-helper/d' "$rootfs/etc/doas.conf"
    chown 0:0 "$rootfs/etc/doas.conf"
    chmod 400 "$rootfs/etc/doas.conf"
fi

cat > "$rootfs/etc/udev/rules.d/99-nexa-automount.rules" << 'RULES_EOF'
# Mount removable USB/SD drives under /media when plugged in, and clean up
# when they are pulled. nexa-brokerd re-checks that the device really is
# a removable USB/SD drive before touching it.
ACTION=="add", SUBSYSTEM=="block", KERNEL=="sd[a-z]*|mmcblk[0-9]*", RUN+="/usr/local/bin/nexa-brokerd drive-added %k"
ACTION=="remove", SUBSYSTEM=="block", KERNEL=="sd[a-z]*|mmcblk[0-9]*", RUN+="/usr/local/bin/nexa-brokerd drive-removed %k"
RULES_EOF

echo "nexa-automount staged into $rootfs"
