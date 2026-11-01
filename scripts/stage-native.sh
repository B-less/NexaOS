#!/bin/sh
# Stage the native (Rust) tools from build/native into the rootfs, plus the
# package trust store: /etc/nexa/keys holds the public keys of publishers
# whose .nexa packages install without a warning (assets/keys/*.pub; the
# matching secret keys never leave the publisher's machine).
set -eu
rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}
for bin in "$base"/build/native/*; do
    [ -f "$bin" ] || continue
    install -D -m 0755 "$bin" "$rootfs/usr/local/bin/$(basename "$bin")"
    echo "staged $(basename "$bin")"
done
mkdir -p "$rootfs/etc/nexa/keys"
for key in "$base"/assets/keys/*.pub; do
    [ -f "$key" ] && install -m 0644 "$key" "$rootfs/etc/nexa/keys/$(basename "$key")"
done
chmod 0755 "$rootfs/etc/nexa/keys"

# Windows Boot Repair (nexa-bootfix, Rust): finds why Windows won't start,
# fixes the safe parts after a backup, saves a report to a USB drive. Root
# through doas: no password on the live USB; an installed system asks for
# one (diskinstall.PASSWORD_HELPERS).
if [ -x "$rootfs/usr/local/bin/nexa-bootfix" ]; then
    printf '%s\n' \
        '#!/bin/sh' \
        '# Opens nexa-bootfix in a terminal, as root.' \
        '. /usr/local/lib/nexa-wayland-env.sh' \
        'exec foot --title "Windows Boot Repair" --window-size-chars 110x40 doas /usr/local/bin/nexa-bootfix' \
        > "$rootfs/usr/local/bin/nexa-bootfix-window"
    chmod 0755 "$rootfs/usr/local/bin/nexa-bootfix-window"
    [ -f "$rootfs/etc/doas.conf" ] || : > "$rootfs/etc/doas.conf"
    grep -q '/usr/local/bin/nexa-bootfix$' "$rootfs/etc/doas.conf" \
        || echo 'permit nopass user cmd /usr/local/bin/nexa-bootfix' >> "$rootfs/etc/doas.conf"
    chown 0:0 "$rootfs/etc/doas.conf"
    chmod 400 "$rootfs/etc/doas.conf"
    mkdir -p "$rootfs/usr/share/applications"
    cat > "$rootfs/usr/share/applications/nexa-bootfix.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Windows Boot Repair
Comment=Find why Windows won't start, fix it safely, save a report to a USB drive
Exec=/usr/local/bin/nexa-bootfix-window
Icon=drive-harddisk
Terminal=false
Categories=System;
Keywords=windows;boot;repair;bootmgr;efi;secure boot;
DESKTOP_EOF
fi
