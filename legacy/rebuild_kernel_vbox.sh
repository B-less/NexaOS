#!/bin/bash
set -e
KDIR="/home/nexa/NexaOS/kernel/linux-6.18.33"
BASE="/home/nexa/NexaOS"

echo "=== Enabling VirtualBox/VESA framebuffer in kernel ==="

cd "$KDIR"

# Enable the required framebuffer drivers
./scripts/config --enable CONFIG_FB_VESA
./scripts/config --enable CONFIG_DRM_BOCHS
./scripts/config --enable CONFIG_FB_SIMPLE
./scripts/config --enable CONFIG_SYSFB_SIMPLEFB
./scripts/config --enable CONFIG_FB_BOOT_VESA_SUPPORT

# Disable DRM_FBDEV_EMULATION dependency issues
./scripts/config --enable CONFIG_DRM_FBDEV_EMULATION

# Regenerate config without prompting
make olddefconfig

echo "=== Starting partial kernel recompile (bzImage only) ==="
make -j$(nproc) bzImage 2>&1 | tail -5

echo "=== Kernel rebuilt ==="
ls -lh arch/x86/boot/bzImage
