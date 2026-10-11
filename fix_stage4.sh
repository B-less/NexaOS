#!/bin/bash
set -e
BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"

umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT

# Fix tcc wrapper (tcc.real already exists from previous run)
cat > $MNT/usr/bin/tcc << 'EOF'
#!/bin/sh
exec /usr/bin/tcc.real -B /usr/lib/tcc "$@"
EOF
chmod +x $MNT/usr/bin/tcc

# Rewrite pkg cleanly
cat > $MNT/bin/pkg << 'EOF'
#!/bin/sh
PKG_CACHE="/var/cache/pkg"
PKG_DB="/var/lib/pkg/installed"
PKG_INDEX="$PKG_CACHE/index.txt"
mkdir -p "$PKG_CACHE" "$(dirname "$PKG_DB")"
[ -f "$PKG_DB" ] || touch "$PKG_DB"
case "$1" in
    update)
        echo "Fetching package index..."
        wget -qO "$PKG_INDEX" "http://10.0.2.2:8080/packages/index.txt" 2>/dev/null \
            && echo "Index updated." || echo "Could not reach package server."
        ;;
    available)
        echo "Available packages:"
        if [ -f "$PKG_INDEX" ]; then
            grep -v '^#' "$PKG_INDEX" | awk -F: '{printf "  %-20s %s\n",$1,$3}'
        else
            echo "  (run 'pkg update' first)"
        fi
        ;;
    list)
        echo "Installed packages:"
        if [ -s "$PKG_DB" ]; then
            cat "$PKG_DB" | while read l; do echo "  $l"; done
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
        grep -q "^$2$" "$PKG_DB" 2>/dev/null && echo "$2 is already installed." && exit 0
        LINE=$(grep "^$2:" "$PKG_INDEX" 2>/dev/null)
        [ -z "$LINE" ] && echo "Package '$2' not found. Run 'pkg update'." && exit 1
        URL=$(echo "$LINE" | cut -d: -f2)
        echo "Downloading $2..."
        wget -qO /tmp/_pkg.tar.gz "$URL" \
            && tar xzf /tmp/_pkg.tar.gz -C / \
            && rm -f /tmp/_pkg.tar.gz \
            && echo "$2" >> "$PKG_DB" \
            && echo "$2 installed successfully."
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

umount $MNT
echo "tcc wrapper and pkg fixed successfully."
