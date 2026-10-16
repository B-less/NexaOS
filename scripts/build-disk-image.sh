#!/bin/sh
# Populate an ext4 disk image directly from a rootfs directory tree, without
# mounting anything and without host root. Alpine packages leave some files
# root-owned and unreadable-by-design (e.g. busybox-suid's bbsuid), which a
# non-root `mke2fs -d` cannot read, so this always runs as root in a container.
set -eu

ROOTFS_DIR=$1
IMG=$2
SIZE=$3

apk info -e e2fsprogs >/dev/null || apk add --no-cache e2fsprogs >/dev/null  # in nexa-build already

# Embed the kernel and initramfs in the rootfs itself, not just the ISO's own
# /boot (build-iso.sh). Without this, a system booted directly via QEMU
# -kernel/-initrd (as opposed to the live ISO, which still has them on the
# mounted CD) has no copy of its own boot files anywhere on disk, and the
# installer (nexa-setup) has nothing to copy onto the target machine's /boot.
DIST_DIR=$(dirname "$IMG")
mkdir -p "$ROOTFS_DIR/boot"
cp "$DIST_DIR/bzImage" "$ROOTFS_DIR/boot/vmlinuz"
cp "$DIST_DIR/initramfs.cpio.gz" "$ROOTFS_DIR/boot/initramfs.cpio.gz"

# rootfs/ is extracted and staged mostly by the host user, so most of it shows
# up here as uid/gid 1000 -- the same ids as the desktop "user" account, and
# mke2fs -d bakes them into the image. That handed "user" ownership of /etc,
# /usr/bin, /usr/local/bin (home of the passwordless doas helpers) and so on,
# i.e. a free path to root. Everything outside user's home belongs to root;
# other service ids (messagebus, seatd...) are left alone. The two deliberate
# exceptions are restored right after. (-user/-group, not -uid/-gid: this
# runs under BusyBox find in the build container.)
find "$ROOTFS_DIR" -xdev -path "$ROOTFS_DIR/home/user" -prune -o \
    \( -user 1000 -o -group 1000 \) -exec chown -h 0:0 {} +
chown -R 1000:1000 "$ROOTFS_DIR/home/user"
# Settings drops a new wallpaper in here without a privileged helper.
[ -d "$ROOTFS_DIR/usr/share/backgrounds/nexaos" ] && chown -R 1000:1000 "$ROOTFS_DIR/usr/share/backgrounds/nexaos"
# nexa-ai-gui (running as "user") needs to read the api_key; see stage-nexa-ai.sh.
[ -f "$ROOTFS_DIR/etc/nexa-ai/config.json" ] && chown 0:1000 "$ROOTFS_DIR/etc/nexa-ai/config.json"
# Password checks from the desktop account (the lock screen, doas) go through
# PAM's unix_chkpwd, which reads /etc/shadow via the "shadow" group (gid 42).
# Set last: the chown pass above resets groups and a chown clears setgid.
chown 0:42 "$ROOTFS_DIR/etc/shadow"
chmod 640 "$ROOTFS_DIR/etc/shadow"
if [ -f "$ROOTFS_DIR/usr/sbin/unix_chkpwd" ]; then
    chown 0:42 "$ROOTFS_DIR/usr/sbin/unix_chkpwd"
    chmod 2755 "$ROOTFS_DIR/usr/sbin/unix_chkpwd"
fi

rm -f "$IMG"
# mke2fs creates the backing file itself when it doesn't exist yet, sized from
# the trailing size argument (accepts K/M/G suffixes) — no qemu-img needed.
mke2fs -F -t ext4 -L nexaos -d "$ROOTFS_DIR" "$IMG" "$SIZE"
# Container root owns the new file; open it up so the host user (and QEMU) can
# read/write it without needing host root.
chmod 666 "$IMG"
