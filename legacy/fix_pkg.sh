#!/bin/bash
set -e
BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"

umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT

# ── overwrite pkg with our live version ──────────────────────────────────────
cat > $MNT/bin/pkg << 'PKGEOF'
#!/bin/sh
PKG_CACHE="/var/cache/pkg"
PKG_DB="/var/lib/pkg/installed"
PKG_INDEX="$PKG_CACHE/index.txt"
mkdir -p "$PKG_CACHE" "$(dirname $PKG_DB)"
[ -f "$PKG_DB" ] || touch "$PKG_DB"

_fetch_index() {
    wget -qO "$PKG_INDEX" "http://10.0.2.2:8080/packages/index.txt" 2>/dev/null \
        && echo "Remote index updated." && return 0
    echo "Note: No remote server. Using local repo."
    return 1
}

case "$1" in
    update)
        echo "Fetching package index..."
        _fetch_index || true
        ;;
    available)
        echo "Available packages:"
        if [ -f "$PKG_INDEX" ]; then
            grep -v '^#' "$PKG_INDEX" | awk -F: '{printf "  %-20s %s\n",$1,$3}'
        else
            echo "  (run 'pkg update' to fetch remote list)"
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
        [ -f "$PKG_INDEX" ] && grep -i "$2" "$PKG_INDEX" | awk -F: '{printf "  %-20s %s\n",$1,$3}' || echo "No remote index. Run: pkg update"
        ;;
    install)
        [ -z "$2" ] && echo "Usage: pkg install <name>" && exit 1
        PKG="$2"
        grep -q "^$PKG$" "$PKG_DB" 2>/dev/null && echo "$PKG is already installed." && exit 0
        LINE=$(grep "^$PKG:" "$PKG_INDEX" 2>/dev/null)
        [ -z "$LINE" ] && echo "Package '$PKG' not found. Run: pkg update" && exit 1
        URL=$(echo "$LINE" | cut -d: -f2)
        echo "Downloading $PKG..."
        wget -qO /tmp/_pkg.tar.gz "$URL" && tar xzf /tmp/_pkg.tar.gz -C / && rm -f /tmp/_pkg.tar.gz
        echo "$PKG" >> "$PKG_DB"
        echo "$PKG installed."
        ;;
    remove)
        [ -z "$2" ] && echo "Usage: pkg remove <name>" && exit 1
        sed -i "/^$2$/d" "$PKG_DB"
        echo "$2 removed."
        ;;
    info)
        [ -z "$2" ] && echo "Usage: pkg info <name>" && exit 1
        grep "^$2:" "$PKG_INDEX" 2>/dev/null | awk -F: '{print "Name: "$1"\nURL:  "$2"\nDesc: "$3}' || echo "No info found."
        ;;
    *)
        echo "Nexa Package Manager"
        echo "Usage: pkg <command> [args]"
        echo ""
        echo "Commands:"
        echo "  update          Fetch remote package index"
        echo "  available       List available packages"
        echo "  list            List installed packages"
        echo "  search <name>   Search for a package"
        echo "  install <name>  Install a package"
        echo "  remove <name>   Remove a package"
        echo "  info <name>     Show package info"
        ;;
esac
PKGEOF
chmod +x $MNT/bin/pkg

umount $MNT
echo "pkg updated."
