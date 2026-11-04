#!/bin/sh
# Open apps follow a theme change live (a Python Nexa app and the C Video
# Player), in a headless Sway inside the nexa-apptest container.
# Usage: sh scripts/test-theme-live.sh [OUT_DIR]   (screenshots; default dist/theme-test)
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-apptest:${version%.*}
out=${1:-$repo/dist/theme-test}
mkdir -p "$out"
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$(cd "$out" && pwd):/out" -e OUT=/out "$image" sh -c '
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 HOME=/root
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
printf "output * resolution 1280x800\ndefault_border none\nfor_window [app_id=\".*\"] floating enable\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1) WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
rc=0
# The NexaHexa icon sets, installed for the icon-shift test.
mkdir -p $HOME/.local/share/icons && cp -r /src/assets/iconset/NexaHexa /src/assets/iconset/NexaHexa-* $HOME/.local/share/icons/
mkdir -p /usr/local/share/applications
printf "[Desktop Entry]\nType=Application\nName=Files\nExec=true\nIcon=nexa-files\n" > /usr/local/share/applications/nexa-files.desktop
export GSETTINGS_BACKEND=keyfile
for t in headless_theme_live.py headless_vp_theme_live.py headless_icon_shift.py; do
    PYTHONPATH=/src/common python3 -c "from nexa_common import themes; themes.apply(\"horizon\", icon_style=\"colour\", reload=False)" >/dev/null 2>&1
    python3 /src/common/tests/$t 2>&1 > /tmp/out.log 2>&1 || rc=1; grep -E "^(PASS|FAIL)" /tmp/out.log; grep -A8 Traceback /tmp/out.log | head -12
done
exit $rc
'
