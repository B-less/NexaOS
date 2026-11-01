#!/bin/sh
# Drive the Image Viewer window (image-viewer/tests/test_window.py) in a headless
# Sway inside the nexa-apptest container, against the real engine in
# build/native (nexa-imageinfo, nexa-fileops, nexa-thumb), and save a screenshot.
#
# Usage: sh scripts/test-viewer-app.sh [OUT.png]   (default dist/viewer-test.png)
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$repo/dist/viewer-test.png}
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-apptest:${version%.*}
docker image inspect "$image" >/dev/null 2>&1 || { echo "build $image first: sh scripts/test-app-headless.sh"; exit 1; }
[ -x "$repo/build/native/nexa-imageinfo" ] || sh "$repo/scripts/build-native.sh"
mkdir -p "$(dirname "$out")"
outdir=$(cd "$(dirname "$out")" && pwd)

docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$outdir:/out" -e SHOT="/out/$(basename "$out")" \
    -e NEXA_IMAGEINFO=/src/build/native/nexa-imageinfo -e NEXA_FILEOPS=/src/build/native/nexa-fileops -e NEXA_THUMB=/src/build/native/nexa-thumb "$image" sh -c '
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 HOME=/root
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
printf "output * resolution 1280x800 bg #0b1020 solid_color\ndefault_border none\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 || { tail -20 /tmp/sway.log; exit 1; }
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
PYTHONPATH=/src/common python3 -c "from nexa_common import themes; themes.apply(reload=False)" >/dev/null 2>&1 || true
python3 /src/image-viewer/tests/test_window.py 2>/tmp/err.log; rc=$?
grep -E "Traceback|Error|CRITICAL|Gtk-WARNING" -A3 /tmp/err.log | grep -v "xdg-toplevel-icon" | head -30
exit $rc
'
