#!/bin/bash
set -e

BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"
TCC_SRC="/tmp/tcc-out"
PY_SRC="/tmp/micropython-1.23.0/ports/unix"

echo "=== Nexa OS Stage 4 Build ==="

umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT

# -------------------------------------------------------
# 1. TinyCC - C Compiler
# -------------------------------------------------------
echo "[1/3] Installing TinyCC..."
mkdir -p $MNT/usr/bin
mkdir -p $MNT/usr/lib/tcc
mkdir -p $MNT/usr/include

cp $TCC_SRC/bin/tcc $MNT/usr/bin/tcc
chmod +x $MNT/usr/bin/tcc

# Install TCC runtime lib and headers so tcc can actually compile programs
cp -r $TCC_SRC/lib/tcc/* $MNT/usr/lib/tcc/

# Install basic C headers (from our build system)
for h in stdio.h stdlib.h string.h math.h unistd.h errno.h stddef.h \
          stdint.h stdbool.h stdarg.h assert.h ctype.h time.h sys/types.h \
          sys/stat.h fcntl.h signal.h limits.h float.h; do
    dir="$MNT/usr/include/$(dirname $h)"
    mkdir -p "$dir"
    [ -f "/usr/include/$h" ] && cp "/usr/include/$h" "$dir/" 2>/dev/null || true
    [ -f "/usr/lib/x86_64-linux-musl/include/$h" ] && cp "/usr/lib/x86_64-linux-musl/include/$h" "$dir/" 2>/dev/null || true
done

# -------------------------------------------------------
# 2. MicroPython
# -------------------------------------------------------
echo "[2/3] Installing MicroPython..."
cp $PY_SRC/micropython $MNT/usr/bin/python
chmod +x $MNT/usr/bin/python
ln -sf /usr/bin/python $MNT/usr/bin/python3

# -------------------------------------------------------
# 3. Live Package Manager
# -------------------------------------------------------
echo "[3/3] Upgrading pkg to live package manager..."
cat > $MNT/bin/pkg << 'EOF'
#!/bin/sh
PKG_INDEX="http://10.0.2.2:8080/packages/index.txt"
PKG_CACHE="/var/cache/pkg"
PKG_DB="/var/lib/pkg/installed"

mkdir -p "$PKG_CACHE" "$( dirname $PKG_DB )"
[ -f "$PKG_DB" ] || touch "$PKG_DB"

case "$1" in
    update)
        echo "Fetching package index..."
        wget -qO "$PKG_CACHE/index.txt" "$PKG_INDEX" 2>/dev/null && echo "Index updated." || echo "Warning: Could not reach package server. Using local cache."
        ;;
    available)
        if [ -f "$PKG_CACHE/index.txt" ]; then
            echo "Available packages:"
            grep -v '^#' "$PKG_CACHE/index.txt" | awk -F: '{printf "  %-20s %s\n", $1, $3}'
        else
            echo "Run 'pkg update' first to fetch the package list."
        fi
        ;;
    list)
        echo "Installed packages:"
        if [ -s "$PKG_DB" ]; then
            while IFS= read -r line; do echo "  $line"; done < "$PKG_DB"
        else
            echo "  (none)"
        fi
        ;;
    search)
        [ -z "$2" ] && echo "Usage: pkg search <name>" && exit 1
        grep -i "$2" "$PKG_CACHE/index.txt" 2>/dev/null | awk -F: '{printf "  %-20s %s\n", $1, $3}' || echo "No results found."
        ;;
    install)
        [ -z "$2" ] && echo "Usage: pkg install <name>" && exit 1
        PKG="$2"
        if grep -q "^$PKG:" "$PKG_DB" 2>/dev/null; then
            echo "$PKG is already installed."
            exit 0
        fi
        LINE=$(grep "^$PKG:" "$PKG_CACHE/index.txt" 2>/dev/null)
        if [ -z "$LINE" ]; then
            echo "Package '$PKG' not found. Run 'pkg update' first."
            exit 1
        fi
        URL=$(echo "$LINE" | cut -d: -f2)
        echo "Downloading $PKG..."
        TMP="/tmp/pkg_$PKG.tar.gz"
        wget -qO "$TMP" "$URL" && tar xzf "$TMP" -C / && rm -f "$TMP"
        echo "$PKG" >> "$PKG_DB"
        echo "$PKG installed successfully."
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

# Create a hello world test script for TCC
cat > $MNT/home/nexa/hello.c << 'EOF'
#include <stdio.h>
int main() {
    printf("Hello from Nexa OS!\nC compiler is working.\n");
    return 0;
}
EOF

# Create a hello world python script
cat > $MNT/home/nexa/hello.py << 'EOF'
print("Hello from Nexa OS!")
print("Python is working.")
import sys
print("Python version:", sys.version)
EOF

umount $MNT
echo ""
echo "=== Stage 4 Complete! ==="
echo "Commands available:"
echo "  tcc hello.c -o hello && ./hello"
echo "  python hello.py"
echo "  pkg update && pkg available"
