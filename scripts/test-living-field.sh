#!/bin/sh
# The Living Field (Settings' front page) in a headless Sway with the real
# MicroBrain, Nexa Link and Nexa Doctor running. See
# common/tests/headless_living_field.py. Screenshots: dist/living-field-test.
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-apptest:${version%.*}
for b in nexa-microbrain nexa-linkd nexa-doctor; do
    [ -x "$repo/build/native/$b" ] || { echo "build it first: make native"; exit 1; }
done
out=${1:-$repo/dist/living-field-test}
mkdir -p "$out"
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$(cd "$out" && pwd):/out" -e HOME=/root -e OUT=/out "$image" sh -c '
for b in nexa-microbrain nexa-linkd nexa-doctor; do install -m 0755 /src/build/native/$b /usr/local/bin/$b; done
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
export DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address)
printf "output * resolution 1366x768 bg /src/assets/wallpaper.png fill\nfor_window [app_id=\".*\"] floating enable\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1)
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
PYTHONPATH=/src/common python3 -c "from nexa_common import themes; themes.apply(reload=False)" >/dev/null 2>&1
nexa-microbrain serve >/tmp/mb.log 2>&1 &
nexa-linkd serve >/tmp/link.log 2>&1 &
sleep 1.5
python3 /src/common/tests/headless_living_field.py 2>/tmp/err.log; rc=$?
grep -E "Traceback|Error" -A6 /tmp/err.log | grep -v "xdg-toplevel-icon\|Glycin" | head -30
exit $rc
'
