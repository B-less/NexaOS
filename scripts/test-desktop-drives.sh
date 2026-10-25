#!/bin/sh
# USB drives on the desktop, in a headless Sway (nexa-apptest container).
# Usage: sh scripts/test-desktop-drives.sh [OUT_DIR]   (default dist/drives-test)
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
out=${1:-$repo/dist/drives-test}
mkdir -p "$out"
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$(cd "$out" && pwd):/out" -e OUT=/out "nexa-apptest:${version%.*}" sh -c '
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 HOME=/root
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
printf "output * resolution 1280x800 bg /src/assets/wallpaper.png fill\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1) WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
PYTHONPATH=/src/common python3 -c "from nexa_common import themes; themes.apply(\"horizon\", reload=False)" >/dev/null 2>&1
python3 /src/shell/tests/headless_drives.py 2>/tmp/err.log; rc=$?
grep -E "Traceback|Error" -A6 /tmp/err.log | head -20
exit $rc
'
