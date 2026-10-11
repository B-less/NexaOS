#!/bin/sh
# Stage nexa-install, the GTK installer for .nexa application packages, into
# the rootfs. Also registers the custom application/vnd.nexa.package MIME
# type (glob *.nexa) with shared-mime-info, so double-clicking a .nexa file
# in the file manager launches this app via the same Gio.AppInfo association
# system every other file type uses -- see provision-rootfs.sh, which runs
# update-mime-database and update-desktop-database after every stage-*.sh
# script (including this one) has run.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-installer" "$rootfs/usr/local/bin" \
    "$rootfs/usr/share/applications" "$rootfs/usr/share/mime/packages"

rm -rf "$rootfs/usr/local/lib/nexa-installer/nexa_installer"
cp -a "$base/installer/nexa_installer" "$rootfs/usr/local/lib/nexa-installer/nexa_installer"
find "$rootfs/usr/local/lib/nexa-installer/nexa_installer" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

printf '%s\n' \
    '#!/bin/sh' \
    '. /usr/local/lib/nexa-wayland-env.sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-installer:/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_installer "$@"' \
    > "$rootfs/usr/local/bin/nexa-install"
chmod +x "$rootfs/usr/local/bin/nexa-install"

printf '%s\n' \
    '#!/bin/sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-installer:${PYTHONPATH:-}" python3 -m nexa_installer.control_helper' \
    > "$rootfs/usr/local/bin/nexa-install-helper"
chmod +x "$rootfs/usr/local/bin/nexa-install-helper"

if [ ! -f "$rootfs/etc/doas.conf" ]; then
    : > "$rootfs/etc/doas.conf"
fi
if ! grep -q nexa-install-helper "$rootfs/etc/doas.conf" 2>/dev/null; then
    echo 'permit nopass user cmd /usr/local/bin/nexa-install-helper' >> "$rootfs/etc/doas.conf"
fi
chown 0:0 "$rootfs/etc/doas.conf"
chmod 400 "$rootfs/etc/doas.conf"

cat > "$rootfs/usr/share/mime/packages/nexa-package.xml" << 'MIME_EOF'
<?xml version="1.0" encoding="UTF-8"?>
<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
  <mime-type type="application/vnd.nexa.package">
    <comment>NexaOS application package</comment>
    <glob pattern="*.nexa"/>
  </mime-type>
</mime-info>
MIME_EOF

cat > "$rootfs/usr/share/applications/nexa-installer.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Nexa Installer
Comment=Install .nexa application packages
Exec=/usr/local/bin/nexa-install %f
Icon=nexa-installer
Terminal=false
Categories=System;PackageManager;
MimeType=application/vnd.nexa.package;
DESKTOP_EOF

echo "nexa-installer staged into $rootfs"
