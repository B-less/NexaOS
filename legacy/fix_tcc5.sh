#!/bin/bash
set -e
BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"

umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT

echo "[1/2] Copying musl headers into /usr/lib/tcc/include..."
# Copy all musl headers — they're clean and self-contained (no glibc bits/ deps)
cp -r /usr/include/x86_64-linux-musl/. $MNT/usr/lib/tcc/include/

echo "[2/2] Updating tcc wrapper..."
cat > $MNT/usr/bin/tcc << 'EOF'
#!/bin/sh
exec /usr/bin/tcc.real -B /usr/lib/tcc -L /usr/lib/x86_64-linux-gnu -nostdinc -I /usr/lib/tcc/include -static "$@"
EOF
chmod +x $MNT/usr/bin/tcc

echo "=== include dir now has ==="
ls $MNT/usr/lib/tcc/include/ | head -20
echo "..."

umount $MNT
echo "Done."
