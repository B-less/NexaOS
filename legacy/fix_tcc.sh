#!/bin/bash
set -e
BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"
umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT
cat > $MNT/usr/bin/tcc << 'EOF'
#!/bin/sh
exec /usr/bin/tcc.real -B /usr/lib/tcc -static "$@"
EOF
chmod +x $MNT/usr/bin/tcc
umount $MNT
echo "tcc wrapper fixed."
