#!/bin/bash
set -e
BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"

umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT

echo "[1/3] Installing musl dynamic linker..."
mkdir -p $MNT/lib/x86_64-linux-musl
cp /usr/lib/x86_64-linux-musl/libc.so $MNT/lib/x86_64-linux-musl/libc.so
ln -sf x86_64-linux-musl/libc.so $MNT/lib/ld-musl-x86_64.so.1

echo "[2/3] Installing static-compiled tcc..."
cp /tmp/tinycc-static/tcc $MNT/usr/bin/tcc.real
chmod +x $MNT/usr/bin/tcc.real

# Also install fresh tcc runtime libs
cp -r /tmp/tinycc-static/lib/tcc/* $MNT/usr/lib/tcc/ 2>/dev/null || true

echo "[3/3] Updating tcc wrapper (add -static flag)..."
cat > $MNT/usr/bin/tcc << 'EOF'
#!/bin/sh
exec /usr/bin/tcc.real -B /usr/lib/tcc -static "$@"
EOF
chmod +x $MNT/usr/bin/tcc

umount $MNT
echo "Done. tcc is now fully operational."
