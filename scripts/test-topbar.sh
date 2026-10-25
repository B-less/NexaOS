#!/bin/sh
# The top bar's "Hide when not in use" (nexa-barhide, C) in a headless Sway
# with the real bar (waybar) and NexaOS's bar config, inside nexa-apptest:
# hidden, it takes no space; the pointer resting on the top edge brings it
# in; moving back down hides it again; "Always shown" keeps its space.
# Usage: sh scripts/test-topbar.sh [OUT_DIR]   (screenshots; default dist/topbar-test)
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-apptest:${version%.*}
docker image inspect "$image" >/dev/null 2>&1 || { echo "build $image first: sh scripts/test-app-headless.sh"; exit 1; }
[ -x "$repo/build/native/nexa-barhide" ] || { echo "build it first: make native"; exit 1; }
[ -f "$repo/rootfs/etc/xdg/waybar/config" ] || { echo "no rootfs bar config yet: make iso once"; exit 1; }
out=${1:-$repo/dist/topbar-test}
mkdir -p "$out"
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$(cd "$out" && pwd):/out" -e HOME=/root "$image" sh -c '
apk info -e waybar >/dev/null || apk add --no-cache -q waybar >/dev/null 2>&1 || { echo "no waybar"; exit 1; }
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
# The bar needs a session bus, as in a real session (dbus-run-session there).
export DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address)
printf "output * resolution 1280x800 bg /src/assets/wallpaper.png fill\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1)
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
PYTHONPATH=/src/common python3 -c "from nexa_common import themes; themes.apply(reload=False)" >/dev/null 2>&1
P=/usr/share/wlr-protocols/unstable/wlr-virtual-pointer-unstable-v1.xml
wayland-scanner client-header $P /tmp/wlr-virtual-pointer-unstable-v1-client-protocol.h
wayland-scanner private-code $P /tmp/vp-protocol.c
gcc -O2 -I/tmp -o /tmp/vpointer /src/common/tests/vpointer.c /tmp/vp-protocol.c $(pkg-config --cflags --libs wayland-client) || exit 1
rc=0
pass() { echo "PASS $1"; }
fail() { echo "FAIL $1${2:+  [$2]}"; rc=1; }
top() { swaymsg -t get_workspaces | python3 -c "import json,sys; print([w for w in json.load(sys.stdin) if w[\"focused\"]][0][\"rect\"][\"y\"])"; }
# How different the top 30 px are from the bare desktop (the first shot):
# mean difference per pixel byte, 0 = the same.
band() { grim -g "0,0 1280x30" -t ppm /tmp/b.ppm && python3 -c "
cur=open(\"/tmp/b.ppm\",\"rb\").read().split(b\"\n\",3)[3]
import os
ref=open(\"/tmp/ref.ppm\",\"rb\").read().split(b\"\n\",3)[3] if os.path.exists(\"/tmp/ref.ppm\") else cur
print(sum(abs(a-b) for a,b in zip(cur,ref))*10//len(cur))"; }
mkfifo /tmp/vp.in; /tmp/vpointer < /tmp/vp.in > /tmp/vp.out & exec 3>/tmp/vp.in
move() { echo "move $1 $2" >&3; sleep 0.1; }

base=/src/rootfs/etc/xdg/waybar/config
sed "s/\"layer\": \"top\",/\"layer\": \"top\", \"exclusive\": false, \"start_hidden\": true,/" $base > /tmp/autohide.json
waybar -c /tmp/autohide.json >/tmp/waybar.log 2>&1 &
/src/build/native/nexa-barhide 38 >/tmp/barhide.log 2>&1 &
sleep 3
move 640 400; sleep 0.5
[ "$(top)" -lt 10 ] && pass "hidden, the bar takes no space (windows can use the whole height)" || fail "no space taken" "workspace y $(top)"
grim -g "0,0 1280x30" -t ppm /tmp/ref.ppm; hidden=$(band); grim /out/1-hidden.png
move 640 300; move 640 120; move 640 0; sleep 0.6
shown=$(band); grim /out/2-shown.png
[ "$shown" -gt 40 ] && pass "resting on the top edge brings the bar in" || fail "bar shown" "$hidden vs $shown"
move 640 400; sleep 0.4; after=$(band)
[ "$after" -lt 10 ] && pass "moving back down hides it again (even a fast jump)" || fail "hidden again" "$after vs $hidden"
move 300 0; sleep 0.05; move 300 400; sleep 0.5; flick=$(band)
[ "$flick" -lt 10 ] && pass "brushing past the edge doesn'"'"'t bring it in" || fail "brush" "$flick vs $hidden"
grim /out/3-hidden-again.png

pkill -x waybar; pkill -x nexa-barhide; sleep 0.5
waybar -c $base >/tmp/waybar2.log 2>&1 &
sleep 3
[ "$(top)" -ge 30 ] && pass "Always shown: the bar keeps its space ($(top) px)" || fail "always shown" "workspace y $(top)"
grim /out/4-always.png
exit $rc
'
