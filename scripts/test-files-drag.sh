#!/bin/sh
# Several selected files dragged onto a folder with a real mouse, in the
# grid and the list: all must move. See file-manager/tests/headless_multi_drag.py.
# Headless Sway inside the nexa-apptest container.
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-apptest:${version%.*}
docker image inspect "$image" >/dev/null 2>&1 || { echo "build $image first: sh scripts/test-app-headless.sh"; exit 1; }
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -e HOME=/root -e NEXA_FILEOPS=/src/build/native/nexa-fileops -e DRAG_DEBUG="${DRAG_DEBUG:-}" "$image" sh -c '
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
printf "output * resolution 1280x800\ndefault_border none\nfor_window [app_id=\".*\"] floating enable\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1)
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
P=/usr/share/wlr-protocols/unstable/wlr-virtual-pointer-unstable-v1.xml
wayland-scanner client-header $P /tmp/wlr-virtual-pointer-unstable-v1-client-protocol.h
wayland-scanner private-code $P /tmp/vp-protocol.c
gcc -Wall -Wextra -Werror -O2 -I/tmp -o /tmp/vpointer /src/common/tests/vpointer.c /tmp/vp-protocol.c $(pkg-config --cflags --libs wayland-client) || exit 1
python3 /src/file-manager/tests/headless_multi_drag.py 2>/tmp/err.log; rc=$?
grep -E "Traceback|Error|DEBUG" -A5 /tmp/err.log | head -20
exit $rc
'
