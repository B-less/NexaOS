#!/bin/sh
# Stage the NexaOS shell: start menu (nexa-start), Control Centre
# (nexa-control), the theme engine CLI (nexa-theme) and the bar's
# notification bell script.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/usr/local/lib/nexa-shell" "$rootfs/usr/local/bin"
rm -rf "$rootfs/usr/local/lib/nexa-shell/nexa_shell"
cp -a "$base/shell/nexa_shell" "$rootfs/usr/local/lib/nexa-shell/nexa_shell"
find "$rootfs/usr/local/lib/nexa-shell/nexa_shell" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

for panel in start control dock; do
    printf '%s\n' \
        '#!/bin/sh' \
        '. /usr/local/lib/nexa-wayland-env.sh' \
        "exec env PYTHONPATH=\"/usr/local/lib/nexa-shell:/usr/local/lib/nexa-common:\${PYTHONPATH:-}\" python3 -m nexa_shell $panel" \
        > "$rootfs/usr/local/bin/nexa-$panel"
    chmod +x "$rootfs/usr/local/bin/nexa-$panel"
done

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
