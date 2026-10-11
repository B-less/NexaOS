#!/bin/sh
# Build a bootable BIOS/El-Torito ISO (isolinux + xorriso) that VirtualBox
# (and any other BIOS-mode VM/PC) can boot directly as an optical disc.
#
# It ships the exact same kernel, initramfs, and rootfs (nexa.img) as a
# direct-disk boot, just with nexa.img embedded as a plain file on the ISO
# instead of attached as a virtio-blk/SATA disk. /init's live-CD fallback
# (see build-initramfs.sh) finds it there, loop-mounts it read-only, and
# layers a tmpfs on top via overlayfs -- a normal writable-feeling session
# that simply discards changes on reboot, like any other live CD.
#
# Runs as root inside a throwaway Alpine container (same no-host-sudo
# pattern as the rest of the build): xorriso/syslinux are build-time-only
# tools, never installed in the shipped OS.
set -eu

DIST_DIR=$1
ISO_OUTPUT=$2

apk add --no-cache xorriso syslinux >/dev/null

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT

mkdir -p "$STAGE/boot/isolinux"
cp "$DIST_DIR/bzImage" "$STAGE/boot/vmlinuz"
cp "$DIST_DIR/initramfs.cpio.gz" "$STAGE/boot/initramfs.cpio.gz"
cp "$DIST_DIR/nexa.img" "$STAGE/nexa.img"

for f in isolinux.bin ldlinux.c32; do
    src=""
    for d in /usr/share/syslinux /usr/lib/syslinux/bios; do
        [ -f "$d/$f" ] && src="$d/$f" && break
    done
    if [ -z "$src" ]; then
        echo "ERROR: $f not found (syslinux package layout changed?)" >&2
        exit 1
    fi
    cp "$src" "$STAGE/boot/isolinux/$f"
done

cat > "$STAGE/boot/isolinux/isolinux.cfg" << 'CFG_EOF'
DEFAULT nexa
TIMEOUT 50
PROMPT 1

LABEL nexa
  MENU LABEL NexaOS (live)
  KERNEL /boot/vmlinuz
  APPEND initrd=/boot/initramfs.cpio.gz console=tty0 console=ttyS0
CFG_EOF

mkdir -p "$(dirname "$ISO_OUTPUT")"
rm -f "$ISO_OUTPUT"
xorriso -as mkisofs \
    -V "NEXAOS" \
    -b boot/isolinux/isolinux.bin \
    -c boot/isolinux/boot.cat \
    -no-emul-boot -boot-load-size 4 -boot-info-table \
    -o "$ISO_OUTPUT" \
    "$STAGE"

echo "[OK] ISO created: $ISO_OUTPUT ($(du -h "$ISO_OUTPUT" | cut -f1))"
