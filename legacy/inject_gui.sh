#!/bin/bash
set -e
BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"

umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT

echo "1. Copying linux headers for tcc..."
cp -r /usr/include/linux $MNT/usr/lib/tcc/include/ || true
cp -r /usr/include/asm-generic $MNT/usr/lib/tcc/include/ || true
cp -r /usr/include/x86_64-linux-gnu/asm $MNT/usr/lib/tcc/include/ || true

echo "2. Setting up desktop source..."
mkdir -p $MNT/home/nexa/desktop
cp $BASE/desktop.c $MNT/home/nexa/desktop/
cp $BASE/font8x8.h $MNT/home/nexa/desktop/font8x8_basic.h
chown -R 1000:1000 $MNT/home/nexa/desktop

echo "3. Updating gui run script..."
mkdir -p $MNT/apps/desktop-gui
cat > $MNT/apps/desktop-gui/run << 'EOF'
#!/bin/sh
clear
echo "============================="
echo " Starting Nexa OS GUI..."
echo "============================="
if [ ! -c /dev/fb0 ]; then
    echo "Error: /dev/fb0 not found."
    echo "Make sure you booted with './boot.sh gui'"
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
echo "Desktop GUI successfully injected."
