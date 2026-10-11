#!/bin/sh
# Stage the nexa-calc GTK calculator into the rootfs.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-calc" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-calc/nexa_calc"
cp -a "$base/calculator/nexa_calc" "$rootfs/usr/local/lib/nexa-calc/nexa_calc"
find "$rootfs/usr/local/lib/nexa-calc/nexa_calc" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-calc:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_calc "$@"' \
    > "$rootfs/usr/local/bin/nexa-calc"
chmod +x "$rootfs/usr/local/bin/nexa-calc"

cat > "$rootfs/usr/share/applications/nexa-calc.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Calculator
Comment=Basic arithmetic calculator
Exec=/usr/local/bin/nexa-calc
Icon=nexa-calc
Terminal=false
Categories=Utility;
DESKTOP_EOF

echo "nexa-calc staged into $rootfs"
