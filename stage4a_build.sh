#!/bin/bash
set -e
BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"
TCC_SRC="/tmp/tcc-out"

umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT

echo "[1/2] Installing TinyCC..."
mkdir -p $MNT/usr/bin $MNT/usr/lib/tcc $MNT/usr/include/sys

cp $TCC_SRC/bin/tcc $MNT/usr/bin/tcc
chmod +x $MNT/usr/bin/tcc

cp -r $TCC_SRC/lib/tcc/* $MNT/usr/lib/tcc/

for h in stdio.h stdlib.h string.h math.h unistd.h errno.h stddef.h stdint.h \
          stdbool.h stdarg.h assert.h ctype.h time.h limits.h float.h; do
    [ -f "/usr/include/$h" ] && cp /usr/include/$h $MNT/usr/include/$h 2>/dev/null || true
done
for h in types.h stat.h; do
    [ -f "/usr/include/sys/$h" ] && cp /usr/include/sys/$h $MNT/usr/include/sys/$h 2>/dev/null || true
done
for h in fcntl.h signal.h; do
    [ -f "/usr/include/$h" ] && cp /usr/include/$h $MNT/usr/include/$h 2>/dev/null || true
done

echo "[2/2] Installing live pkg manager..."
cat > $MNT/bin/pkg << 'EOF'
#!/bin/sh
PKG_CACHE="/var/cache/pkg"
PKG_DB="/var/lib/pkg/installed"
PKG_INDEX="$PKG_CACHE/index.txt"

mkdir -p "$PKG_CACHE" "$(dirname $PKG_DB)"
[ -f "$PKG_DB" ] || touch "$PKG_DB"

case "$1" in
    update)
        echo "Fetching package index..."
        wget -qO "$PKG_INDEX" "http://10.0.2.2:8080/packages/index.txt" 2>/dev/null \
            && echo "Index updated." \
            || echo "Warning: Could not reach package server."
        ;;
    available)
        [ ! -f "$PKG_INDEX" ] && echo "Run 'pkg update' first." && exit 1
        echo "Available packages:"
        grep -v '^#' "$PKG_INDEX" | awk -F: '{printf "  %-20s %s\n", $1, $3}'
        ;;
    list)
        echo "Installed packages:"
        if [ -s "$PKG_DB" ]; then
            while IFS= read -r l; do echo "  $l"; done < "$PKG_DB"
        else
            echo "  (none)"
        fi
        ;;
    search)
        [ -z "$2" ] && echo "Usage: pkg search <name>" && exit 1
        grep -i "$2" "$PKG_INDEX" 2>/dev/null | awk -F: '{printf "  %-20s %s\n",$1,$3}' || echo "No results."
        ;;
    install)
        [ -z "$2" ] && echo "Usage: pkg install <name>" && exit 1
        PKG="$2"
        grep -q "^$PKG$" "$PKG_DB" 2>/dev/null && echo "$PKG already installed." && exit 0
        LINE=$(grep "^$PKG:" "$PKG_INDEX" 2>/dev/null)
        [ -z "$LINE" ] && echo "Package '$PKG' not found. Run 'pkg update'." && exit 1
        URL=$(echo "$LINE" | cut -d: -f2)
        echo "Downloading $PKG..."
        TMP="/tmp/pkg_$PKG.tar.gz"
        wget -qO "$TMP" "$URL" && tar xzf "$TMP" -C / && rm -f "$TMP"
        echo "$PKG" >> "$PKG_DB"
        echo "$PKG installed."
        ;;
    remove)
        [ -z "$2" ] && echo "Usage: pkg remove <name>" && exit 1
        sed -i "/^$2$/d" "$PKG_DB"
        echo "$2 removed."
        ;;
    *)
        echo "Usage: pkg <update|available|list|search|install|remove> [args]"
        ;;
esac
EOF
chmod +x $MNT/bin/pkg

# Create test files
cat > $MNT/home/nexa/hello.c << 'EOF'
#include <stdio.h>
int main() {
    printf("Hello from Nexa OS!\nC compiler is working.\n");
    return 0;
}
EOF

cat > $MNT/home/nexa/hello.py << 'EOF'
print("Hello from Nexa OS!")
print("Python is working.")
import sys
print("Python version:", sys.version)
EOF

umount $MNT
echo "TinyCC + pkg manager injected OK."
