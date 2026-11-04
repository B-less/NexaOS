#!/bin/sh
# Stage the NexaOS shell's Python parts: Nexus (nexa-nexus, also nexa-start
# and nexa-launchpad, the old names), the desktop parts (nexa-parts), the
# theme engine CLI (nexa-theme) and the bar's notification bell script.
# Orbit and Deck are C (tools-c), staged with the native tools.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-shell" "$rootfs/usr/local/bin"
rm -rf "$rootfs/usr/local/lib/nexa-shell/nexa_shell"
cp -a "$base/shell/nexa_shell" "$rootfs/usr/local/lib/nexa-shell/nexa_shell"
find "$rootfs/usr/local/lib/nexa-shell/nexa_shell" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

# ("foryou" is the desktop card; nexa-desktop is the session launcher, see
# generate-configs.sh - never reuse that name.)
for panel in nexus start launchpad foryou drives linknotify parts; do
    printf '%s\n' \
        '#!/bin/sh' \
        '. /usr/local/lib/nexa-wayland-env.sh' \
        "exec env PYTHONPATH=\"/usr/local/lib/nexa-shell:/usr/local/lib/nexa-common:\${PYTHONPATH:-}\" python3 -m nexa_shell $panel" \
        > "$rootfs/usr/local/bin/nexa-$panel"
    chmod +x "$rootfs/usr/local/bin/nexa-$panel"
done

# The Dock and the Control Centre were Python panels; Orbit and Deck (C)
# replaced them. nexa-control stays as a name for Deck (older bar configs,
# muscle memory); the rootfs is reused by "make refresh", so the Dock's
# launcher is removed here.
rm -f "$rootfs/usr/local/bin/nexa-dock"
printf '%s\n' '#!/bin/sh' 'exec nexa-deck "$@"' > "$rootfs/usr/local/bin/nexa-control"
chmod +x "$rootfs/usr/local/bin/nexa-control"

mkdir -p "$rootfs/usr/share/applications"
# An entry from older builds (Nexus replaced it, and the name was Apple's):
# the rootfs is reused by "make refresh", so it has to be removed here.
rm -f "$rootfs/usr/share/applications/nexa-launchpad.desktop"
cat > "$rootfs/usr/share/applications/nexa-nexus.desktop" << 'DESKTOP_EOF'
[Desktop Entry]
Type=Application
Name=Nexus
Comment=Your apps, search, settings and Nexa in one place (Super+Space)
Exec=/usr/local/bin/nexa-nexus
Icon=nexa-nexus
Terminal=false
Categories=System;
Keywords=apps;launcher;start;menu;search;honeycomb;
DESKTOP_EOF

printf '%s\n' \
    '#!/bin/sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -m nexa_common.themes "$@"' \
    > "$rootfs/usr/local/bin/nexa-theme"
chmod +x "$rootfs/usr/local/bin/nexa-theme"

# Applies the saved keyboard layout in a Hyprland session (sway reads it
# from its config.d include instead).
printf '%s\n' \
    '#!/bin/sh' \
    'exec env PYTHONPATH="/usr/local/lib/nexa-common:${PYTHONPATH:-}" python3 -c "from nexa_common import compositor, sysconf; compositor.set_keyboard_layout(sysconf.read_keyboard_layout())"' \
    > "$rootfs/usr/local/bin/nexa-keyboard-layout"
chmod +x "$rootfs/usr/local/bin/nexa-keyboard-layout"

cp "$base/shell/nexa-bar-notify" "$rootfs/usr/local/bin/nexa-bar-notify"
chmod +x "$rootfs/usr/local/bin/nexa-bar-notify"

echo "nexa-shell staged into $rootfs"
