#!/bin/bash
set -e
BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"

umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT

cat > $MNT/bin/pkg << 'PKGEOF'
#!/bin/sh
PKG_CACHE="/var/cache/pkg"
PKG_DB="/var/lib/pkg/installed"
PKG_INDEX="$PKG_CACHE/index.txt"
PKG_SERVER="http://10.0.2.2:8080"

mkdir -p "$PKG_CACHE" "$(dirname $PKG_DB)"
[ -f "$PKG_DB" ] || touch "$PKG_DB"

case "$1" in
    update)
        echo "Fetching package index..."
        wget -qO "$PKG_INDEX" "$PKG_SERVER/index.txt" 2>/dev/null \
            && echo "Index updated." \
            || echo "Could not reach package server at $PKG_SERVER"
        ;;
    available)
        echo "Available packages:"
        if [ -f "$PKG_INDEX" ]; then
            grep -v '^#' "$PKG_INDEX" | grep -v '^$' | awk -F: '{printf "  %-20s %s\n",$1,$3}'
        else
            echo "  (run 'pkg update' first)"
        fi
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
        [ ! -f "$PKG_INDEX" ] && echo "Run 'pkg update' first." && exit 1
        grep -i "$2" "$PKG_INDEX" | grep -v '^#' | awk -F: '{printf "  %-20s %s\n",$1,$3}'
        ;;
    install)
        [ -z "$2" ] && echo "Usage: pkg install <name>" && exit 1
        PKG="$2"
        grep -q "^$PKG$" "$PKG_DB" 2>/dev/null && echo "$PKG is already installed." && exit 0
        [ ! -f "$PKG_INDEX" ] && echo "Run 'pkg update' first." && exit 1
        LINE=$(grep "^$PKG:" "$PKG_INDEX" 2>/dev/null)
        [ -z "$LINE" ] && echo "Package '$PKG' not found." && exit 1
        URL=$(echo "$LINE" | cut -d: -f2)
        echo "Downloading $PKG from $URL..."
        wget -qO /tmp/_pkg.tar.gz "$URL" \
            && tar xzf /tmp/_pkg.tar.gz -C / \
            && rm -f /tmp/_pkg.tar.gz \
            && echo "$PKG" >> "$PKG_DB" \
            && echo "$PKG installed successfully." \
            || echo "Failed to download $PKG."
        ;;
    remove)
        [ -z "$2" ] && echo "Usage: pkg remove <name>" && exit 1
        sed -i "/^$2$/d" "$PKG_DB"
        echo "$2 removed."
        ;;
    info)
        [ -z "$2" ] && echo "Usage: pkg info <name>" && exit 1
        [ ! -f "$PKG_INDEX" ] && echo "Run 'pkg update' first." && exit 1
        grep "^$2:" "$PKG_INDEX" | awk -F: '{print "Name: "$1"\nDesc: "$3}' || echo "Not found."
        ;;
    *)
        echo "Nexa Package Manager  (server: $PKG_SERVER)"
        echo ""
        echo "  pkg update              Fetch package index"
        echo "  pkg available           List all packages"
        echo "  pkg list                List installed packages"
        echo "  pkg search <name>       Search packages"
        echo "  pkg install <name>      Install a package"
        echo "  pkg remove <name>       Remove a package"
        echo "  pkg info <name>         Show package details"
        ;;
esac
PKGEOF

chmod +x $MNT/bin/pkg
umount $MNT
echo "pkg fixed with correct server URL."
