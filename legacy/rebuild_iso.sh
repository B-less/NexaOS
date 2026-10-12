#!/bin/bash
set -e

BASE="$(pwd)"
ISO_DIR="$BASE/iso_build"
INIT_DIR="$BASE/init_build"
MNT="$BASE/mnt"

echo "=== Rebuilding ISO with fixed Live CD init ==="

# Fix the /init script to work as Live CD (no disk mounting)
cat > "$INIT_DIR/init" << 'INITEOF'
#!/bin/sh

# Mount essential filesystems
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mkdir -p /dev/pts
mount -t devpts devpts /dev/pts

# Populate devices
echo /sbin/mdev > /proc/sys/kernel/hotplug
mdev -s

# Auto-create fb0 if virtio-gpu is present
if [ ! -e /dev/fb0 ] && [ -f /sys/class/graphics/fb0/dev ]; then
    fbdev="$(cat /sys/class/graphics/fb0/dev 2>/dev/null)"
    major="${fbdev%:*}"
    minor="${fbdev#*:}"
    [ -n "$major" ] && mknod /dev/fb0 c "$major" "$minor" 2>/dev/null
fi

chmod 666 /dev/fb0 2>/dev/null || true

# Mount tmpfs for writable areas
mount -t tmpfs tmpfs /tmp
mount -t tmpfs tmpfs /var/run
mount -t tmpfs tmpfs /var/log

# Create necessary runtime dirs
mkdir -p /var/run /var/log /var/lock

# We ARE the root - no switch_root needed for Live CD
# Just exec the real init
exec /sbin/init
INITEOF

chmod +x "$INIT_DIR/init"

echo "Repacking initramfs..."
cd "$INIT_DIR"
find . -print0 | cpio --null -ov --format=newc 2>/dev/null | gzip -9 > "$ISO_DIR/initramfs.cpio.gz"
echo "Initramfs repacked."

echo "Regenerating ISO..."
cd "$BASE"
rm -f NexaOS.iso
xorriso -as mkisofs \
  -o NexaOS.iso \
  -b isolinux/isolinux.bin \
  -c isolinux/boot.cat \
  -no-emul-boot -boot-load-size 4 -boot-info-table \
  "$ISO_DIR"

echo ""
echo "=== NexaOS.iso rebuilt successfully! ==="
ls -lh NexaOS.iso
