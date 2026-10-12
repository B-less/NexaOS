#!/bin/bash
set -e
BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"

umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT

cat > $MNT/apps/desktop-gui/run << 'EOF'
#!/bin/sh
clear
echo "============================="
echo " Starting Nexa OS GUI..."
echo "============================="

# Auto-create fb0 if mdev missed it
if [ ! -e /dev/fb0 ] && [ -f /sys/class/graphics/fb0/dev ]; then
    fbdev="$(cat /sys/class/graphics/fb0/dev 2>/dev/null)"
    major="${fbdev%:*}"
    minor="${fbdev#*:}"
    if [ -n "$major" ] && [ -n "$minor" ]; then
        mknod /dev/fb0 c "$major" "$minor" 2>/dev/null
    fi
fi

if [ ! -c /dev/fb0 ]; then
    echo "Error: /dev/fb0 not found."
    echo "Make sure you booted with ./boot.sh gui"
    exit 1
fi

if [ ! -c /dev/input/mice ]; then
    echo "Warning: /dev/input/mice not found. Mouse may not work."
fi

cd /home/nexa/desktop
if [ ! -x ./desktop ] || [ desktop.c -nt desktop ]; then
    echo "Compiling desktop.c natively using tcc..."
    tcc desktop.c -o desktop
    if [ $? -ne 0 ]; then
        echo "Compilation failed!"
        exit 1
    fi
fi

echo "Launching desktop..."
./desktop
EOF
chmod +x $MNT/apps/desktop-gui/run

umount $MNT
echo "Updated gui run script."
