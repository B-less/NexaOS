#!/bin/sh
# Populate an ext4 disk image directly from a rootfs directory tree, without
# mounting anything and without host root. Alpine packages leave some files
# root-owned and unreadable-by-design (e.g. busybox-suid's bbsuid), which a
# non-root `mke2fs -d` cannot read, so this always runs as root in a container.
set -eu

ROOTFS_DIR=$1
IMG=$2
SIZE=$3

apk add --no-cache e2fsprogs >/dev/null

# Embed the kernel and initramfs in the rootfs itself, not just the ISO's own
# /boot (build-iso.sh). Without this, a system booted directly via QEMU
# -kernel/-initrd (as opposed to the live ISO, which still has them on the
# mounted CD) has no copy of its own boot files anywhere on disk, and the
# installer (nexa-setup) has nothing to copy onto the target machine's /boot.
DIST_DIR=$(dirname "$IMG")
mkdir -p "$ROOTFS_DIR/boot"
cp "$DIST_DIR/bzImage" "$ROOTFS_DIR/boot/vmlinuz"
cp "$DIST_DIR/initramfs.cpio.gz" "$ROOTFS_DIR/boot/initramfs.cpio.gz"

rm -f "$IMG"
# mke2fs creates the backing file itself when it doesn't exist yet, sized from
# the trailing size argument (accepts K/M/G suffixes) — no qemu-img needed.
mke2fs -F -t ext4 -L nexaos -d "$ROOTFS_DIR" "$IMG" "$SIZE"
# Container root owns the new file; open it up so the host user (and QEMU) can
# read/write it without needing host root.
chmod 666 "$IMG"
