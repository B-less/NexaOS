#!/bin/sh
# The top bar's "Hide when not in use" (nexa-barhide, C) in a headless Sway
# with the real bar (waybar) and NexaOS's bar config, inside nexa-apptest:
# hidden, it takes no space; the pointer resting on the top edge brings it
# in; moving back down hides it again; "Always shown" keeps its space.
# The motion (a snapshot of the strip, drawn on an overlay): sampled while
# it runs, the top of the screen passes through in-between states both
# ways, ends exactly hidden or shown, and with "motion": "none" in
# Interface DNA it just switches.
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
# The top strip, sampled back to back while it changes ("a b c ...");
# each sample is also kept (/tmp/s-N.ppm) for a picture of the motion.
sample() { i=0; while [ $i -lt 14 ]; do v=$(band); cp /tmp/b.ppm /tmp/s-$i.ppm; echo $v; i=$((i+1)); done | tr "\n" " "; }
# How many samples are part-way between hidden (about 0) and shown ($1).
between() { echo "$2" | tr " " "\n" | awk -v top="$1" "\$1 != \"\" && \$1 > 12 && \$1 < top * 0.85 { n++ } END { print n + 0 }"; }
# The sample closest to half-way, as a PNG (no image tools here).
keep() { python3 - "$1" "$2" "$3" <<PYEOF
import sys, zlib, struct
top, seq, out = float(sys.argv[1]), [int(x) for x in sys.argv[2].split()], sys.argv[3]
i = min(range(len(seq)), key=lambda j: abs(seq[j] - top / 2))
raw = open("/tmp/s-%d.ppm" % i, "rb").read().split(b"\n", 3)
w, h = map(int, raw[1].split()); px = raw[3]
rows = b"".join(b"\0" + px[y * w * 3:(y + 1) * w * 3] for y in range(h))
chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
open(out, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))
PYEOF
}
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
move 640 300; move 640 120; move 640 0
showing=$(sample); sleep 0.3
shown=$(band); grim /out/2-shown.png
[ "$shown" -gt 40 ] && pass "resting on the top edge brings the bar in" || fail "bar shown" "$hidden vs $shown"
n=$(between "$shown" "$showing")
[ "$n" -ge 1 ] && pass "it comes in with a motion ($n in-between frames sampled)" || fail "show animation" "samples: $showing (shown $shown)"
keep "$shown" "$showing" /out/2a-showing.png
move 640 400
hiding=$(sample); sleep 0.2; after=$(band)
[ "$after" -lt 10 ] && pass "moving back down hides it again (even a fast jump), the overlay gone" || fail "hidden again" "$after vs $hidden"
n=$(between "$shown" "$hiding")
[ "$n" -ge 1 ] && pass "and leaves with a motion ($n in-between frames sampled)" || fail "hide animation" "samples: $hiding"
keep "$shown" "$hiding" /out/3a-hiding.png
move 300 0; sleep 0.05; move 300 400; sleep 0.5; flick=$(band)
[ "$flick" -lt 10 ] && pass "brushing past the edge doesn'"'"'t bring it in" || fail "brush" "$flick vs $hidden"
grim /out/3-hidden-again.png

# Interface DNA "motion": "none": no animation, the bar just switches.
mkdir -p /root/.config/nexa/interface
echo "{\"motion\": \"none\", \"version\": 1}" > /root/.config/nexa/interface/global.json
move 640 120; move 640 0
quick=$(sample); sleep 0.2
move 640 400
quick2=$(sample); sleep 0.2; still=$(band)
n=$(( $(between "$shown" "$quick") + $(between "$shown" "$quick2") ))
[ "$n" = 0 ] && [ "$still" -lt 10 ] && echo "$quick" | tr " " "\n" | awk "\$1 > 40 { f=1 } END { exit !f }" \
  && pass "with motion none it switches without in-between frames" || fail "motion none" "show: $quick hide: $quick2"
rm -f /root/.config/nexa/interface/global.json
grep -iE "error|fail" /tmp/barhide.log && fail "barhide said something" || true

pkill -x waybar; pkill -x nexa-barhide; sleep 0.5
waybar -c $base >/tmp/waybar2.log 2>&1 &
sleep 3
[ "$(top)" -ge 30 ] && pass "Always shown: the bar keeps its space ($(top) px)" || fail "always shown" "workspace y $(top)"
grim /out/4-always.png
exit $rc
'
