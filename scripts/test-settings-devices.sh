#!/bin/sh
# Settings > Display, Sound, Mouse & Touchpad, Wi-Fi, Notifications,
# Default Apps, Accessibility and Time & Keyboard in a headless Sway, the
# Display page against the real nexa-display. See
# common/tests/headless_settings_devices.py. Screenshots: dist/settings-devices-test.
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-apptest:${version%.*}
[ -x "$repo/build/native/nexa-display" ] || { echo "build it first: sh scripts/build-c-tools.sh"; exit 1; }
out=${1:-$repo/dist/settings-devices-test}
mkdir -p "$out"
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$(cd "$out" && pwd):/out" -e HOME=/root -e OUT=/out "$image" sh -c '
install -m 0755 /src/build/native/nexa-display /usr/local/bin/nexa-display
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
export DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address)
printf "output * resolution 1280x800 bg /src/assets/wallpaper.png fill\nfor_window [app_id=\".*\"] floating enable\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1)
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
PYTHONPATH=/src/common python3 -c "from nexa_common import themes; themes.apply(reload=False)" >/dev/null 2>&1
python3 /src/common/tests/headless_settings_devices.py 2>/tmp/err.log; rc=$?
grep -E "Traceback|Error" -A6 /tmp/err.log | grep -v "xdg-toplevel-icon\|Glycin" | head -30
exit $rc
'
