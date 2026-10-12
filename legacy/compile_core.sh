#!/bin/bash
set -e

BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"

echo "Compiling init and httpd..."
cc -Wall -O2 -static -o $BASE/init_bin $BASE/init.c
cc -Wall -O2 -static -o $BASE/httpd_bin $BASE/httpd.c

echo "Mounting disk..."
mkdir -p "$MNT"
mount -o loop "$BASE/nexa.img" "$MNT"

echo "Installing binaries..."
cp $BASE/init_bin "$MNT/sbin/init"
cp $BASE/httpd_bin "$MNT/sbin/httpd"

echo "Configuring inittab and rcS..."
cat > "$MNT/etc/inittab" << 'EOF'
tty0:1:respawn:/sbin/getty -L tty0 115200 vt100
ttyS0:1:respawn:/sbin/getty -L ttyS0 115200 vt100
httpd:1:respawn:/sbin/httpd
EOF

mkdir -p "$MNT/etc/init.d"
cat > "$MNT/etc/init.d/rcS" << 'EOF'
#!/bin/sh
mount -a
hostname nexa
ifconfig lo 127.0.0.1
ifconfig eth0 up
udhcpc -i eth0 -b >/dev/null 2>&1 &
EOF
chmod +x "$MNT/etc/init.d/rcS"

cat > "$MNT/etc/fstab" << 'EOF'
proc /proc proc defaults 0 0
sysfs /sys sysfs defaults 0 0
devtmpfs /dev devtmpfs defaults 0 0
EOF

# Move old init to nexa-admin
cp $BASE/init "$MNT/sbin/nexa-admin"
chmod +x "$MNT/sbin/nexa-admin"

echo "Unmounting..."
umount "$MNT"

echo "Updating initramfs..."
# The new initramfs init just mounts /dev/sda and switch_roots
cat > "$BASE/initramfs_init" << 'EOF'
#!/bin/sh
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
[ -b /dev/sda ] || mknod /dev/sda b 8 0
mkdir -p /mnt/root
echo "Mounting root filesystem..."
if mount -t ext4 /dev/sda /mnt/root; then
    echo "Switching root..."
    exec switch_root /mnt/root /sbin/init
else
    echo "ERROR: Failed to mount /dev/sda"
    exec /bin/sh
fi
EOF
chmod +x "$BASE/initramfs_init"

# Rebuild initramfs
cd "$BASE/rootfs"
cp "$BASE/initramfs_init" init
find . -print0 | cpio --null -ov --format=newc | gzip -9 > "$BASE/initramfs.cpio.gz"
cd "$BASE"

echo "Core OS built!"
