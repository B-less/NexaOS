#!/bin/bash
# Disk installation helper - handles mounting and APK installation
set -e

DIST_DIR="dist"
MNT_DIR="mnt"
ROOTFS_DIR="rootfs"

echo "[*] NexaOS Disk Installation Helper"
echo ""

# Check if running with sudo
if [[ $EUID -ne 0 ]]; then
   echo "ERROR: This script must be run with sudo"
   echo "Usage: sudo bash scripts/install-disk.sh"
   exit 1
fi

if [[ ! -f "$DIST_DIR/nexa.img" ]]; then
    echo "ERROR: Disk image not found: $DIST_DIR/nexa.img"
    exit 1
fi

echo "[1/6] Mounting disk image..."
if ! mount -o loop "$DIST_DIR/nexa.img" "$MNT_DIR" 2>/dev/null; then
    echo "ERROR: Could not mount disk. Trying to unmount any existing mounts..."
    umount "$MNT_DIR" 2>/dev/null || true
    mount -o loop "$DIST_DIR/nexa.img" "$MNT_DIR"
fi
echo "✓ Disk mounted"

# Cleanup function
cleanup() {
    echo "[*] Cleaning up mounts..."
    umount "$MNT_DIR/proc" 2>/dev/null || true
    umount "$MNT_DIR/sys" 2>/dev/null || true
    umount "$MNT_DIR/dev" 2>/dev/null || true
    umount "$MNT_DIR" 2>/dev/null || true
    echo "✓ Cleanup complete"
}

trap cleanup EXIT

echo "[2/6] Copying rootfs to disk..."
cp -ax "$ROOTFS_DIR"/* "$MNT_DIR/" 2>&1 | grep -v "Operation not permitted" || true
echo "✓ Rootfs copied"

echo "[3/6] Setting up chroot environment..."
mkdir -p "$MNT_DIR"/{dev,proc,sys,run}
mount -t proc proc "$MNT_DIR/proc" 2>/dev/null || true
mount -t sysfs sysfs "$MNT_DIR/sys" 2>/dev/null || true
mount -t devtmpfs devtmpfs "$MNT_DIR/dev" 2>/dev/null || true
echo "✓ Chroot ready"

echo "[4/6] Updating APK package manager..."
chroot "$MNT_DIR" /sbin/apk update 2>&1 | head -3
echo "✓ APK updated"

echo "[5/6] Installing packages..."
echo "  Installing core packages..."
chroot "$MNT_DIR" /sbin/apk add --no-scripts \
    alpine-base busybox util-linux e2fsprogs libcap \
    2>&1 | tail -3 || true

echo "  Installing Wayland/Sway..."
chroot "$MNT_DIR" /sbin/apk add --no-scripts \
    wayland libxkbcommon mesa libdrm libxwayland \
    sway swaybg swaylock swayidle waybar wofi \
    2>&1 | tail -3 || true

echo "  Installing terminal and utilities..."
chroot "$MNT_DIR" /sbin/apk add --no-scripts \
    foot tmux \
    grim slurp xdg-desktop-portal-wlr xwayland \
    2>&1 | tail -3 || true

echo "✓ Packages installed"

echo "[6/6] Configuring system..."
bash scripts/generate-configs.sh "$ROOTFS_DIR" "$MNT_DIR"
echo "✓ Configuration complete"

echo ""
echo "=========================================="
echo "✓ Disk installation COMPLETE!"
echo "=========================================="
echo ""
echo "Disk image: $(ls -lh $DIST_DIR/nexa.img | awk '{print $5, $9}')"
echo ""
echo "Next steps:"
echo "  make boot      - Test serial console"
echo "  make boot-gui  - Test Sway desktop"
