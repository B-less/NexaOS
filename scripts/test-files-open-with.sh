#!/bin/sh
# Click to focus, and Open With that can't end up behind the file manager,
# in a real compositor (headless Sway inside the nexa-apptest container).
# See file-manager/tests/headless_open_with.py. The Sway config tested is
# NexaOS's own, taken from scripts/generate-configs.sh.
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-apptest:${version%.*}
docker image inspect "$image" >/dev/null 2>&1 || { echo "build $image first: sh scripts/test-app-headless.sh"; exit 1; }
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -e HOME=/root "$image" sh -c '
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
# NexaOS sway config, exactly as generate-configs.sh writes it
sed -n "/<< '"'"'SWAY_CONFIG_EOF'"'"'/,/^SWAY_CONFIG_EOF\$/p" /src/scripts/generate-configs.sh | sed "1d;\$d" > /tmp/nexa-sway.conf
mkdir -p /usr/share/backgrounds/nexaos && cp /src/assets/wallpaper.png /usr/share/backgrounds/nexaos/  # as installed
echo "== NexaOS sway config: sway -C"
sway -C -c /tmp/nexa-sway.conf 2>&1 | grep -E "Error|error" | grep -vE "Unable to (open|find)|swaybg|session|seat|backend" || true
sway -C -c /tmp/nexa-sway.conf >/dev/null 2>&1 && echo "   valid" || { echo "   INVALID"; sway -C -c /tmp/nexa-sway.conf 2>&1 | tail -5; exit 1; }
# the test session: its focus settings, nothing else that needs a real desktop
{ printf "output * resolution 1280x800\ndefault_border none\nfor_window [app_id=\".*\"] floating enable\n"
  grep -E "^focus_follows_mouse " /tmp/nexa-sway.conf; } > /tmp/sway.conf
grep -E "^focus_" /tmp/sway.conf | sed "s/^/   uses: /"
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
# both sockets: the IPC one (swaymsg) appears a moment after the Wayland one
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* $XDG_RUNTIME_DIR/sway-ipc.* >/dev/null 2>&1 && break; sleep 0.2; done
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1)
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
P=/usr/share/wlr-protocols/unstable/wlr-virtual-pointer-unstable-v1.xml
wayland-scanner client-header $P /tmp/wlr-virtual-pointer-unstable-v1-client-protocol.h
wayland-scanner private-code $P /tmp/vp-protocol.c
gcc -Wall -Wextra -Werror -O2 -I/tmp -o /tmp/vpointer /src/common/tests/vpointer.c /tmp/vp-protocol.c $(pkg-config --cflags --libs wayland-client) || exit 1
rc=0
python3 /src/file-manager/tests/headless_open_with.py 2>/tmp/err.log || rc=1
grep -E "Traceback|Error" -A5 /tmp/err.log | head -20
exit $rc
'
