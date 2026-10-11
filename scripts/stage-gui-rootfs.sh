#!/bin/sh
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}

mkdir -p "$rootfs/apps/gui" "$rootfs/apps/desktop-gui" "$rootfs/Desktop"
mkdir -p "$rootfs/etc" "$rootfs/data" "$rootfs/usr/local/bin" "$rootfs/usr/share/applications"

cp "$base/gui/nexa-fbdesktop" "$rootfs/apps/desktop-gui/fbdesktop"
cp "$base/gui/nexa_wrapper.sh" "$rootfs/apps/desktop-gui/nexa_wrapper.sh"
cp "$base/gui/build_apps/"* "$rootfs/apps/gui/"
chmod +x "$rootfs/apps/desktop-gui/fbdesktop" "$rootfs/apps/desktop-gui/nexa_wrapper.sh" "$rootfs/apps/gui/"*

printf '%s\n' \
    '#!/bin/sh' \
    'APP_DIR=/disk/apps/desktop-gui' \
    'if [ ! -d /sys/class/graphics/fb0 ]; then echo "No framebuffer driver is active. Boot with GUI video enabled."; exit 1; fi' \
    'if [ ! -e /dev/fb0 ]; then fbdev=$(cat /sys/class/graphics/fb0/dev 2>/dev/null); major=${fbdev%:*}; minor=${fbdev#*:}; [ -n "$major" ] && [ -n "$minor" ] && mkdir -p /dev && mknod /dev/fb0 c "$major" "$minor" 2>/dev/null || true; fi' \
    'if [ ! -x "$APP_DIR/fbdesktop" ]; then echo "Framebuffer desktop binary not found."; exit 1; fi' \
    'exec "$APP_DIR/fbdesktop"' \
    > "$rootfs/apps/desktop-gui/run"
chmod +x "$rootfs/apps/desktop-gui/run"

printf '%s\n' \
    '.txt=/disk/apps/gui/editor' \
    '.c=/disk/apps/gui/editor' \
    '.h=/disk/apps/gui/editor' \
    '.sh=/disk/apps/gui/editor' \
    '.ppm=/disk/apps/gui/image' \
    '.bmp=/disk/apps/gui/image' \
    '.tar=/disk/apps/gui/archive' \
    '.csv=/disk/apps/gui/csv_view' \
    '.md=/disk/apps/gui/md_view' \
    '.wav=/disk/apps/gui/audio' \
    > "$rootfs/etc/associations.conf"

write_icon() {
    name=$1
    exec_path=$2
    icon=$3
    file=$4
    printf '%s\n' "Name=$name" "Exec=$exec_path &" "Icon=$icon" > "$rootfs/Desktop/$file.desktop"
}

write_icon TERMINAL /disk/apps/gui/term 0 Terminal
write_icon FILES /disk/apps/gui/files 3 Files
write_icon CALC /disk/apps/gui/calc 2 Calc
write_icon SYSTEM /disk/apps/gui/sysmon 4 System
write_icon EDITOR /disk/apps/gui/editor 5 Editor
write_icon CLOCK /disk/apps/gui/clock 1 Clock
write_icon TODO /disk/apps/gui/todo 6 Todo
write_icon AUDIO /disk/apps/gui/audio 3 Audio
write_icon ABOUT /disk/apps/gui/about 7 About

printf '%s\n' '# NexaOS GUI Sample' 'Open this from the Files app to verify Markdown associations.' > "$rootfs/data/readme.md"
printf '%s\n' 'NAME,TYPE,STATUS' 'Calculator,GUI,Ready' 'Files,GUI,Ready' 'Editor,GUI,Ready' > "$rootfs/data/apps.csv"

printf '%s\n' '#!/bin/sh' 'exec /disk/apps/desktop-gui/run' > "$rootfs/usr/local/bin/nexa-fbdesktop"
chmod +x "$rootfs/usr/local/bin/nexa-fbdesktop"
printf '%s\n' \
    '[Desktop Entry]' \
    'Type=Application' \
    'Name=Nexa Framebuffer Desktop' \
    'Exec=/usr/local/bin/nexa-fbdesktop' \
    'Terminal=false' \
    'Categories=System;' \
    > "$rootfs/usr/share/applications/nexa-fbdesktop.desktop"

ln -sfn . "$rootfs/disk"

mkdir -p "$rootfs/usr/share/backgrounds/nexaos"
if [ -f "$base/assets/wallpaper.png" ]; then
    cp "$base/assets/wallpaper.png" "$rootfs/usr/share/backgrounds/nexaos/wallpaper.png"
fi
# Owned by "user" (numeric id: this build container's own /etc/passwd has no
# "user" entry) so Settings can drop in a new wallpaper without a privileged
# helper — it's a personal preference, not a security-sensitive file.
chown -R 1000:1000 "$rootfs/usr/share/backgrounds/nexaos"

# Shared by every GTK app's launcher wrapper (nexa-ai-gui, nexa-files, ...):
# GDK defaults to X11 and fails ("cannot open display") unless told otherwise,
# and /etc/profile.d/wayland.sh only fires for login shells, which `sway exec`
# and `su -c` don't use. Sway also doesn't always pick wayland-0 (picks the
# first free wayland-N), so find the socket that actually exists rather than
# assuming its name.
mkdir -p "$rootfs/usr/local/lib"
printf '%s\n' \
    'export GDK_BACKEND=wayland' \
    'export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/1000}"' \
    'if [ -z "${WAYLAND_DISPLAY:-}" ]; then' \
    '    for sock in "$XDG_RUNTIME_DIR"/wayland-[0-9]*; do' \
    '        case "$sock" in *.lock) continue ;; esac' \
    '        [ -S "$sock" ] && WAYLAND_DISPLAY=$(basename "$sock") && break' \
    '    done' \
    'fi' \
    'export WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}"' \
    > "$rootfs/usr/local/lib/nexa-wayland-env.sh"

# Shared library imported by every Nexa GTK app (nexa_common.titlebar gives
# each window its minimize/maximize/close buttons) -- staged once here so
# each app's wrapper just adds it to PYTHONPATH instead of vendoring a copy.
rm -rf "$rootfs/usr/local/lib/nexa-common/nexa_common"
mkdir -p "$rootfs/usr/local/lib/nexa-common"
cp -a "$base/common/nexa_common" "$rootfs/usr/local/lib/nexa-common/nexa_common"
find "$rootfs/usr/local/lib/nexa-common/nexa_common" -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true

echo "GUI staged into $rootfs"
