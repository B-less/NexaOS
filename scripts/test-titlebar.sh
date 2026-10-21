#!/bin/sh
# The window buttons (minimize / maximize / close) clicked with the
# compositor's own pointer in a headless Sway, using NexaOS's window rules
# (every window floats), inside the nexa-apptest container.
# Usage: sh scripts/test-titlebar.sh [OUT_DIR]   (screenshots; default dist/titlebar-test)
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-apptest:${version%.*}
docker image inspect "$image" >/dev/null 2>&1 || { echo "build $image first: sh scripts/test-app-headless.sh"; exit 1; }

out=${1:-$repo/dist/titlebar-test}
mkdir -p "$out"
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$(cd "$out" && pwd):/out" -e OUT=/out "$image" sh -c '
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 HOME=/root
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
printf "output * resolution 1280x800\ndefault_border none\ndefault_floating_border normal 2\nfor_window [app_id=\".*\"] floating enable\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/sway-ipc.* $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 || { tail -20 /tmp/sway.log; exit 1; }
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1)
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
# The test-only virtual mouse (C), built here and never shipped.
P=/usr/share/wlr-protocols/unstable/wlr-virtual-pointer-unstable-v1.xml
wayland-scanner client-header $P /tmp/wlr-virtual-pointer-unstable-v1-client-protocol.h
wayland-scanner private-code $P /tmp/vp-protocol.c
gcc -Wall -Wextra -Werror -O2 -I/tmp -o /tmp/vpointer /src/common/tests/vpointer.c /tmp/vp-protocol.c $(pkg-config --cflags --libs wayland-client) || exit 1
python3 /src/common/tests/headless_titlebar_clicks.py 2>/tmp/err.log; rc=$?
grep -E "Traceback|Error" -A5 /tmp/err.log | head -20
echo "--- Video Player (C)"
if [ -x /src/apps/video-player/build/video-player ]; then
    python3 /src/common/tests/headless_vp_titlebar.py 2>/tmp/err2.log || rc=1
    grep -E "Traceback|Error" -A5 /tmp/err2.log | head -20
else
    echo "not built (make nexa-apps)"
fi
exit $rc
'
