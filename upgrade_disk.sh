#!/bin/bash
set -e

BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"

echo "Mounting disk..."
mkdir -p "$MNT"
mount -o loop "$BASE/nexa.img" "$MNT"

echo "Creating standard Linux hierarchy..."
mkdir -p "$MNT/bin" "$MNT/sbin" "$MNT/usr/bin" "$MNT/usr/sbin"
mkdir -p "$MNT/lib" "$MNT/etc" "$MNT/var" "$MNT/tmp"
mkdir -p "$MNT/home/nexa" "$MNT/root"
mkdir -p "$MNT/dev" "$MNT/proc" "$MNT/sys" "$MNT/mnt/root"

echo "Setting up backward compatibility symlink..."
# /disk -> / so old apps looking for /disk/apps find /apps
ln -snf / "$MNT/disk"

echo "Copying busybox..."
cp "$BASE/rootfs/bin/busybox" "$MNT/bin/busybox"
chmod +x "$MNT/bin/busybox"

echo "Installing busybox symlinks..."
chroot "$MNT" /bin/busybox --install -s /bin

echo "Setting up Basic /etc configuration..."
cat > "$MNT/etc/passwd" << 'EOF'
root:x:0:0:root:/root:/bin/sh
nexa:x:1000:1000:Nexa User:/home/nexa:/sbin/nexa-admin
EOF

# Generate real MD5 hash for "password" at deploy-time
HASH=$(openssl passwd -1 password)
printf 'root:%s:19000:0:99999:7:::\n' "$HASH" > "$MNT/etc/shadow"
printf 'nexa:%s:19000:0:99999:7:::\n' "$HASH" >> "$MNT/etc/shadow"

chmod 644 "$MNT/etc/passwd"
chmod 600 "$MNT/etc/shadow"

cat > "$MNT/etc/group" << 'EOF'
root:x:0:
nexa:x:1000:
EOF

echo "Setting permissions..."
chown -R 1000:1000 "$MNT/home/nexa"
chmod 1777 "$MNT/tmp"

echo "Unmounting..."
umount "$MNT"

echo "Upgrade complete!"
