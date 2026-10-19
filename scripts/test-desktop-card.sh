#!/bin/sh
# The desktop "For you" card and Nexus' Suggested row in a headless Sway,
# inside the nexa-apptest container, against the real nexa-patternd
# (synthetic month of use); screenshots go to OUT_DIR.
#
# Usage: sh scripts/test-desktop-card.sh [OUT_DIR]   (default dist/card-test)
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$repo/dist/card-test}
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-apptest:${version%.*}
docker image inspect "$image" >/dev/null 2>&1 || { echo "build $image first: sh scripts/test-app-headless.sh"; exit 1; }
[ -x "$repo/build/native/nexa-patternd" ] || sh "$repo/scripts/build-native.sh"
mkdir -p "$out"
outdir=$(cd "$out" && pwd)

docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$outdir:/out" -e OUT=/out "$image" sh -c '
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 HOME=/root
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
printf "output * resolution 1280x800 bg /src/assets/wallpaper.png fill\ndefault_border none\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 || { tail -20 /tmp/sway.log; exit 1; }
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
PYTHONPATH=/src/common python3 -c "from nexa_common import themes; themes.apply(reload=False)" >/dev/null 2>&1 || true
python3 /src/shell/tests/test_nexus.py >/tmp/nexus.log 2>/tmp/err2.log; rc2=$?
grep -v "^PASS" /tmp/nexus.log; grep -E "Traceback|Error" -A3 /tmp/err2.log | head -10
python3 /src/shell/tests/test_desktop_card.py 2>/tmp/err.log; rc=$?
grep -E "Traceback|Error|CRITICAL|Gtk-WARNING" -A3 /tmp/err.log | grep -v "xdg-toplevel-icon" | head -30
[ $rc2 = 0 ] || exit $rc2
exit $rc
'
