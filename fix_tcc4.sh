#!/bin/bash
set -e
BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"

umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT

echo "=== TCC include dir ==="
ls $MNT/usr/lib/tcc/include/

# Use tcc's own bundled minimal headers — no glibc needed
cat > $MNT/usr/bin/tcc << 'EOF'
#!/bin/sh
exec /usr/bin/tcc.real -B /usr/lib/tcc -L /usr/lib/x86_64-linux-gnu -nostdinc -I /usr/lib/tcc/include -static "$@"
EOF
chmod +x $MNT/usr/bin/tcc

# Also update hello.c to use only tcc's basic stdio
cat > $MNT/home/nexa/hello.c << 'EOF'
#include <stdio.h>
int main(void) {
    printf("Hello from Nexa OS!\n");
    printf("C compiler is working!\n");
    return 0;
}
EOF

umount $MNT
echo "Done. tcc now uses its own bundled headers."
