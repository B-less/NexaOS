#!/bin/bash
# Build a minimal early-boot initramfs: busybox + the switch_root /init script only.
#
# The persistent Wayland/Sway desktop lives entirely on the ext4 disk image
# (see build-disk-image.sh); the initramfs's only job is to find that disk and
# switch_root into it, so it stays a few hundred KB instead of embedding the
# whole ~300MB rootfs (which the initramfs target used to do).
set -euo pipefail

ROOTFS_DIR=$1
OUT=$2
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT

mkdir -p "$STAGE"/{proc,sys,dev/pts,run,tmp,mnt/root,bin,sbin,usr/bin,usr/sbin,lib}

cp -a "$ROOTFS_DIR/lib/ld-musl-x86_64.so.1" "$STAGE/lib/"
ln -s ld-musl-x86_64.so.1 "$STAGE/lib/libc.musl-x86_64.so.1"
cp -a "$ROOTFS_DIR/bin/busybox" "$STAGE/bin/busybox"
ln -sf bin/busybox "$STAGE/sbin/busybox"

link_applets() {
    srcdir=$1; dstsub=$2; relprefix=$3
    find "$ROOTFS_DIR/$srcdir" -maxdepth 1 -type l 2>/dev/null | while read -r link; do
        tgt=$(readlink "$link")
        case "$tgt" in
            */busybox|busybox)
                name=$(basename "$link")
                ln -sf "${relprefix}busybox" "$STAGE/$dstsub/$name"
                ;;
        esac
    done
}
link_applets bin bin ""
link_applets sbin sbin "../bin/"
link_applets usr/bin usr/bin "../../bin/"
link_applets usr/sbin usr/sbin "../../bin/"

# link_applets only harvests *symlinks to busybox* from the full rootfs, so
# it silently drops an applet the moment some other package's real binary
# shadows it there (found by hand: adding util-linux for the installer put
# a real setuid /bin/mount in rootfs/, which replaced busybox's own mount
# symlink -- link_applets then stopped linking "mount" into the initramfs
# at all, since a real ELF file isn't a symlink to match on. This left the
# initramfs unable to mount *anything*, breaking every boot). The
# initramfs's job runs before any real root is mounted, so it must own a
# guaranteed-busybox copy of what it actually calls, independent of
# whatever the full system later installs over the same names.
for applet in mount umount switch_root mkdir mknod cat grep sed cut ls \
              sleep basename losetup mktemp; do
    ln -sf busybox "$STAGE/bin/$applet"
done

cat > "$STAGE/init" << 'INIT_EOF'
#!/bin/sh
PATH=/sbin:/bin:/usr/sbin:/usr/bin

mkdir -p /proc /sys /dev /run /tmp /mnt/root

mount -t proc proc /proc 2>/dev/null || echo "[INIT] WARN: could not mount proc"
mount -t sysfs sysfs /sys 2>/dev/null || echo "[INIT] WARN: could not mount sysfs"
mount -t devtmpfs devtmpfs /dev 2>/dev/null || echo "[INIT] WARN: could not mount devtmpfs"
mkdir -p /dev/pts
mount -t devpts devpts /dev/pts 2>/dev/null || true

if [ -e /proc/sys/kernel/hotplug ]; then
    ( echo /sbin/mdev > /proc/sys/kernel/hotplug ) 2>/dev/null || true
fi
[ -x /sbin/mdev ] && /sbin/mdev -s 2>/dev/null || true

ensure_block_node() {
    DEV="$1"
    [ -b "/dev/$DEV" ] && return 0
    # /sys/block/$DEV only has a "dev" file for a *whole disk* (e.g.
    # /sys/block/sda/dev) -- a partition's is under its parent instead
    # (/sys/block/sda/sda3/dev, not /sys/block/sda3/dev). /sys/class/block
    # is the flat namespace that has every disk AND every partition as a
    # direct entry, so it resolves both uniformly. Found by hand: an
    # installed system on a non-virtio disk (e.g. plain IDE/AHCI) failed to
    # boot because this fallback silently did nothing for the root
    # partition specifically -- devtmpfs/mdev hadn't auto-created its node
    # yet, and this path never found it to mknod it manually either.
    SYSDEV_FILE="/sys/class/block/$DEV/dev"
    if [ -f "$SYSDEV_FILE" ]; then
        MAJOR=$(cut -d: -f1 "$SYSDEV_FILE")
        MINOR=$(cut -d: -f2 "$SYSDEV_FILE")
        echo "[INIT] Creating /dev/$DEV ($MAJOR:$MINOR)"
        mknod "/dev/$DEV" b "$MAJOR" "$MINOR" 2>/dev/null || true
    fi
}

try_mount_root() {
    DEV="$1"
    ensure_block_node "$DEV"
    [ -b "/dev/$DEV" ] || return 1
    echo "[INIT] Attempting to mount /dev/$DEV..."
    if mount -t ext4 "/dev/$DEV" /mnt/root 2>/dev/null; then
        echo "[INIT] Successfully mounted /dev/$DEV"
        if [ -x /mnt/root/sbin/init ]; then
            echo "[INIT] Found /sbin/init, executing switch_root..."
            exec switch_root /mnt/root /sbin/init
        fi
        echo "[INIT] ERROR: /sbin/init not found in /mnt/root"
        umount /mnt/root 2>/dev/null || true
    fi
    return 1
}

echo "[INIT] Waiting for block device..."
TIMEOUT=20
COUNTER=0
while [ $COUNTER -lt $TIMEOUT ]; do
    if grep -qE " (sd|vd|hd|xvd)[a-z]$| nvme[0-9]+n[0-9]+$" /proc/partitions 2>/dev/null; then
        echo "[INIT] Found block device in /proc/partitions"
        break
    fi
    sleep 1
    COUNTER=$((COUNTER + 1))
done

ROOT_FROM_CMDLINE=$(sed -n 's/.*root=\([^ ]*\).*/\1/p' /proc/cmdline 2>/dev/null | sed 's#^/dev/##')
if [ -n "$ROOT_FROM_CMDLINE" ]; then
    echo "[INIT] Trying root= from cmdline: /dev/$ROOT_FROM_CMDLINE"
    try_mount_root "$ROOT_FROM_CMDLINE"
fi

for DEV in vda sda hda xvda nvme0n1; do
    try_mount_root "$DEV"
done

# The whole-disk names above only help when the disk itself IS the root
# filesystem; a normal installed system has root on a *partition*, and the
# device name a partition gets (e.g. vdb3 at install time) is not guaranteed
# to match what it's called on a later boot -- it depends on which bus/slot
# the disk is attached to (seen in practice: the same disk was vdb during
# install, but sda when booted alone under a different QEMU disk
# interface). grub.cfg's root= is only a hint, so on a mismatch, fall back
# to trying every partition of every detected disk. The disk can be visible
# here before the kernel finishes registering its partitions, so retry
# briefly rather than scanning once and giving up.
for SYSDEV in /sys/block/vd* /sys/block/sd* /sys/block/hd* /sys/block/xvd* /sys/block/nvme*n*; do
    [ -e "$SYSDEV" ] || continue
    DEV=$(basename "$SYSDEV")
    case "$DEV" in
        loop*|ram*|sr*|fd*) continue ;;
    esac
    PART_TRIES=0
    while [ "$PART_TRIES" -lt 10 ]; do
        FOUND_PART=
        for PART in "$SYSDEV"/"$DEV"*; do
            [ -e "$PART/dev" ] || continue
            FOUND_PART=1
            try_mount_root "$(basename "$PART")"
        done
        [ -n "$FOUND_PART" ] && break
        PART_TRIES=$((PART_TRIES + 1))
        sleep 0.5
    done
done

# Live-ISO fallback: no direct block-device root was found, which is normal
# when booting from an optical drive (VirtualBox et al present the ISO as a
# CD-ROM, not a disk with an ext4 filesystem on it). The ISO carries the same
# nexa.img this script mounts directly on real disk boots, just embedded as a
# plain file at its root -- loop-mount that file read-only and layer a tmpfs
# on top via overlayfs so the session is normally writable but discards
# changes on reboot, like any other live CD.
echo "[INIT] No direct root device found; trying live-ISO (CD) boot..."
mkdir -p /mnt/cdrom
for CDDEV in sr0 sr1 scd0 hdc hdd; do
    ensure_block_node "$CDDEV"
    [ -b "/dev/$CDDEV" ] || continue
    echo "[INIT] Trying CD-ROM /dev/$CDDEV..."
    if mount -t iso9660 -o ro "/dev/$CDDEV" /mnt/cdrom 2>/dev/null; then
        echo "[INIT] Mounted /dev/$CDDEV"
        break
    fi
done

if [ -f /mnt/cdrom/nexa.img ]; then
    echo "[INIT] Found nexa.img on CD, setting up loop + overlay..."
    mkdir -p /mnt/lower /mnt/live
    # noload: nexa.img's ext4 journal may be "dirty" (not cleanly unmounted --
    # a previous disk boot that was powered off rather than shut down leaves
    # it this way) and a read-only mount can't replay a journal to clean it,
    # so without this a dirty image would refuse to mount at all here. Safe
    # to skip replay: this filesystem is only ever the read-only lower layer
    # of the live overlay below, never written to directly.
    if losetup /dev/loop0 /mnt/cdrom/nexa.img 2>/dev/null \
        && mount -t ext4 -o ro,noload /dev/loop0 /mnt/lower 2>/dev/null; then
        mount -t tmpfs -o size=1G tmpfs /mnt/live
        mkdir -p /mnt/live/upper /mnt/live/work
        if mount -t overlay overlay \
            -o lowerdir=/mnt/lower,upperdir=/mnt/live/upper,workdir=/mnt/live/work \
            /mnt/root 2>/dev/null; then
            echo "[INIT] Live overlay mounted"
            if [ -x /mnt/root/sbin/init ]; then
                echo "[INIT] Found /sbin/init, executing switch_root (live CD)..."
                exec switch_root /mnt/root /sbin/init
            fi
            echo "[INIT] ERROR: /sbin/init not found in live overlay"
        else
            echo "[INIT] ERROR: overlay mount failed"
        fi
    else
        echo "[INIT] ERROR: loop/ext4 mount of nexa.img failed"
    fi
fi

echo "[INIT] ERROR: Could not mount any block device"
ls -la /dev/ | grep -E '^b' || true
echo "[INIT] Dropping to emergency shell..."
exec /bin/sh
INIT_EOF
chmod +x "$STAGE/init"

( cd "$STAGE" && find . -print0 | cpio --null -ov --format=newc 2>/dev/null | gzip -9 ) > "$OUT"
echo "[OK] initramfs: $(du -h "$OUT" | cut -f1)"
