#!/bin/bash
set -e
BASE="/home/nexa/NexaOS"
INIT_DIR="$BASE/init_build"
ISO_DIR="$BASE/iso_build"

echo "=== Injecting new desktop and rebuilding ISO ==="

# Copy new desktop.c into initramfs build
cp /mnt/c/Users/BBF/desktop.c "$INIT_DIR/home/nexa/desktop/desktop.c"

# Remove pre-compiled binary so it recompiles on next boot
rm -f "$INIT_DIR/home/nexa/desktop/desktop"

# Also update the gui run script to delete old binary
cat > "$INIT_DIR/apps/desktop-gui/run" << 'EOF'
#!/bin/sh
clear
echo "============================="
echo " Starting Nexa OS GUI..."
echo "============================="

if [ ! -e /dev/fb0 ] && [ -f /sys/class/graphics/fb0/dev ]; then
    fbdev="$(cat /sys/class/graphics/fb0/dev 2>/dev/null)"
    major="${fbdev%:*}"; minor="${fbdev#*:}"
    [ -n "$major" ] && mknod /dev/fb0 c "$major" "$minor" 2>/dev/null
fi

chmod 666 /dev/fb0 2>/dev/null || true

if [ ! -c /dev/fb0 ]; then
    echo "Error: /dev/fb0 not found."
    echo "Make sure you booted with ./boot.sh gui"
    exit 1
fi

cd /home/nexa/desktop
if [ ! -x ./desktop ] || [ desktop.c -nt ./desktop ]; then
    echo "Compiling desktop..."
    tcc desktop.c -o desktop
    if [ $? -ne 0 ]; then echo "Compilation failed!"; exit 1; fi
fi

echo "Launching desktop..."
./desktop
EOF
chmod +x "$INIT_DIR/apps/desktop-gui/run"

echo "Repacking initramfs..."
cd "$INIT_DIR"
find . -print0 | cpio --null -ov --format=newc 2>/dev/null | gzip -9 > "$ISO_DIR/initramfs.cpio.gz"
echo "Initramfs repacked: $(du -sh $ISO_DIR/initramfs.cpio.gz | cut -f1)"

echo "Regenerating ISO..."
cd "$BASE"
rm -f NexaOS.iso
xorriso -as mkisofs \
  -o NexaOS.iso \
  -b isolinux/isolinux.bin \
  -c isolinux/boot.cat \
  -no-emul-boot -boot-load-size 4 -boot-info-table \
  "$ISO_DIR"

cp NexaOS.iso /mnt/c/Users/BBF/Desktop/NexaOS.iso
ls -lh NexaOS.iso
echo ""
echo "=== ISO rebuilt and copied to Desktop! ==="
