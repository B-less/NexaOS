#!/bin/bash
set -e
BASE="$(pwd)"
ISO_DIR="$BASE/iso_build"
KDIR="$BASE/kernel/linux-6.18.33"

echo "=== Updating ISO with new kernel ==="

# Copy new kernel
cp "$KDIR/arch/x86/boot/bzImage" "$ISO_DIR/bzImage"
echo "Kernel updated: $(ls -lh $ISO_DIR/bzImage | awk '{print $5}')"

# Update ISOLINUX config to add vga=792 for VESA framebuffer (VirtualBox + QEMU)
cat > "$ISO_DIR/isolinux/isolinux.cfg" << 'EOF'
DEFAULT nexaos
TIMEOUT 30
PROMPT 1

LABEL nexaos
  MENU LABEL Boot Nexa OS (Live CD)
  KERNEL /bzImage
  APPEND initrd=/initramfs.cpio.gz vga=792 console=tty0 console=ttyS0 rdinit=/init

LABEL nexaos_safe
  MENU LABEL Boot Nexa OS (Safe Mode - VGA text only)
  KERNEL /bzImage
  APPEND initrd=/initramfs.cpio.gz console=tty0 console=ttyS0 rdinit=/init
EOF

echo "ISOLINUX config updated with vga=792"

# Rebuild ISO
echo "Regenerating ISO..."
cd "$BASE"
rm -f NexaOS.iso
xorriso -as mkisofs \
  -o NexaOS.iso \
  -b isolinux/isolinux.bin \
  -c isolinux/boot.cat \
  -no-emul-boot -boot-load-size 4 -boot-info-table \
  "$ISO_DIR"

cp NexaOS.iso /mnt/c/Users/BBF/Desktop/NexaOS.iso 2>/dev/null || true
echo ""
echo "=== DONE! ISO on Desktop: $(ls -lh /mnt/c/Users/BBF/Desktop/NexaOS.iso | awk '{print $5}') ==="
