#!/bin/sh
# Stage the nexa-settings GTK settings app into the rootfs.
#
# Must run after stage-nexa-ai.sh, which creates /etc/doas.conf — this script
# appends its own permit line rather than overwriting the file, so both
# privileged helpers (nexa-ai's and this one) stay allowed.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-settings" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

rm -rf "$rootfs/usr/local/lib/nexa-settings/nexa_settings"
cp -a "$base/settings-app/nexa_settings" "$rootfs/usr/local/lib/nexa-settings/nexa_settings"
find "$rootfs/usr/local/lib/nexa-settings/nexa_settings" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-settings:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_settings "$@"' \
    > "$rootfs/usr/local/bin/nexa-settings"
chmod +x "$rootfs/usr/local/bin/nexa-settings"

printf '%s\n' \
    '#!/bin/sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-settings:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_settings.helper' \
    > "$rootfs/usr/local/bin/nexa-settings-helper"
chmod +x "$rootfs/usr/local/bin/nexa-settings-helper"

if [ ! -f "$rootfs/etc/doas.conf" ]; then
    : > "$rootfs/etc/doas.conf"
fi
if ! grep -q nexa-settings-helper "$rootfs/etc/doas.conf" 2>/dev/null; then
    echo 'permit nopass user cmd /usr/local/bin/nexa-settings-helper' >> "$rootfs/etc/doas.conf"
fi
chown 0:0 "$rootfs/etc/doas.conf"
chmod 400 "$rootfs/etc/doas.conf"

cat > "$rootfs/usr/share/applications/nexa-settings.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Settings
Comment=Wallpaper, system info, and assistant configuration
Exec=/usr/local/bin/nexa-settings
Icon=nexa-settings
Terminal=false
Categories=Settings;
DESKTOP_EOF

echo "nexa-settings staged into $rootfs"
