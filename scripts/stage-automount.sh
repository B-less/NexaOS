#!/bin/sh
# Stage USB automount: the root helper (nexa-mount-helper), the doas rule the
# file manager's Devices menu uses, and the udev rules that mount removable
# drives under /media on plug-in and clean up on unplug.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-automount" "$rootfs/usr/local/bin" "$rootfs/etc/udev/rules.d" "$rootfs/media"
chown 0:0 "$rootfs/media"
chmod 755 "$rootfs/media"

rm -rf "$rootfs/usr/local/lib/nexa-automount/nexa_automount"
cp -a "$base/automount/nexa_automount" "$rootfs/usr/local/lib/nexa-automount/nexa_automount"
find "$rootfs/usr/local/lib/nexa-automount/nexa_automount" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

# Absolute python path: udev runs this with a minimal environment.
printf '%s\n' \
    '#!/bin/sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-automount:${PYTHONPATH:-}" /usr/bin/python3 -m nexa_automount.helper "$@"' \
    > "$rootfs/usr/local/bin/nexa-mount-helper"
chmod +x "$rootfs/usr/local/bin/nexa-mount-helper"

if [ ! -f "$rootfs/etc/doas.conf" ]; then
    : > "$rootfs/etc/doas.conf"
fi
if ! grep -q nexa-mount-helper "$rootfs/etc/doas.conf" 2>/dev/null; then
    echo 'permit nopass user cmd /usr/local/bin/nexa-mount-helper' >> "$rootfs/etc/doas.conf"
fi
chown 0:0 "$rootfs/etc/doas.conf"
chmod 400 "$rootfs/etc/doas.conf"

cat > "$rootfs/etc/udev/rules.d/99-nexa-automount.rules" << 'RULES_EOF'
# Mount removable USB/SD drives under /media when plugged in, and clean up
# when they are pulled. nexa-mount-helper re-checks that the device really is
# a removable USB/SD drive before touching it.
ACTION=="add", SUBSYSTEM=="block", KERNEL=="sd[a-z]*|mmcblk[0-9]*", RUN+="/usr/local/bin/nexa-mount-helper auto %k"
ACTION=="remove", SUBSYSTEM=="block", KERNEL=="sd[a-z]*|mmcblk[0-9]*", RUN+="/usr/local/bin/nexa-mount-helper cleanup %k"
RULES_EOF

echo "nexa-automount staged into $rootfs"
