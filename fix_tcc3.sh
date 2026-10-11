#!/bin/bash
set -e
BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"

umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT

# Copy musl static libc so tcc can link against it
cp /usr/lib/x86_64-linux-musl/libc.a $MNT/usr/lib/tcc/

# Overwrite tcc wrapper: add -L /usr/lib/tcc so tcc searches there for crt*.o and libc.a
cat > $MNT/usr/bin/tcc << 'EOF'
#!/bin/sh
exec /usr/bin/tcc.real -B /usr/lib/tcc -L /usr/lib/tcc -static "$@"
EOF
chmod +x $MNT/usr/bin/tcc

echo "=== /usr/lib/tcc contents ==="
ls -lh $MNT/usr/lib/tcc/

umount $MNT
echo "Done."
