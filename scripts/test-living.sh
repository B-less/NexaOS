#!/bin/sh
# The Living Visual Engine (nexa-living, C) in a headless Sway (pixman: the
# CPU renderer, as in VMs) with real Nexa apps. Checks that it draws, that
# a Nexa app's window gets its style behind the glass and the desktop its
# own, that an unfocused window holds still, that Static and Off do what
# they say, that a theme change recolours it, that apps' clicks reach it,
# and measures it: CPU, memory, frame time, frame rate per quality.
# Usage: sh scripts/test-living.sh [OUT_DIR]   (default dist/living-test)
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-apptest:${version%.*}
[ -x "$repo/build/native/nexa-living" ] || { echo "build it first: make native"; exit 1; }
out=${1:-$repo/dist/living-test}
mkdir -p "$out"
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$(cd "$out" && pwd):/out" -e HOME=/root "$image" sh -c '
install -m 0755 /src/build/native/nexa-living /usr/local/bin/nexa-living
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
export DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address)
printf "output * resolution 1366x768 bg /src/assets/wallpaper.png fill\nfor_window [app_id=\".*\"] floating enable\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1)
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
export PYTHONPATH=/src/common
python3 -c "from nexa_common import themes; themes.apply(\"horizon\", reload=False)" >/dev/null 2>&1
rc=0
pass() { echo "PASS $1"; }
fail() { echo "FAIL $1${2:+  [$2]}"; rc=1; }
measure() { echo "MEASURE $1"; }
mg() { python3 -c "from nexa_common import magic; $1"; }
stat_of() { python3 -c "import json; print(json.load(open(\"$XDG_RUNTIME_DIR/nexa-living.stats\"))[\"$1\"])" 2>/dev/null || echo none; }
# Mean per-byte difference between two shots inside a rectangle (x y w h).
shot() { grim -t ppm /tmp/$1.ppm; }
diff_in() { python3 - "$@" <<PYEOF
import sys
def load(n):
    raw = open("/tmp/%s.ppm" % n, "rb").read().split(b"\\n", 3)
    w, h = map(int, raw[1].split()); return w, raw[3]
a, b = sys.argv[1], sys.argv[2]; x, y, w, h = map(int, sys.argv[3:7])
W, pa = load(a); _, pb = load(b)
tot = n = 0
for yy in range(y, y + h, 3):
    o = (yy * W + x) * 3
    sa, sb = pa[o:o + w * 3], pb[o:o + w * 3]
    tot += sum(abs(p - q) for p, q in zip(sa[::7], sb[::7])); n += len(sa[::7])
print(round(tot * 100 / max(n, 1)))
PYEOF
}
rect_of() { swaymsg -t get_tree | python3 -c "
import json,sys
def walk(n):
    if n.get(\"app_id\") == \"$1\": r = n[\"rect\"]; print(r[\"x\"], r[\"y\"], r[\"width\"], r[\"height\"]); sys.exit()
    for c in n.get(\"nodes\", []) + n.get(\"floating_nodes\", []): walk(c)
walk(json.load(sys.stdin))"; }

sleep 3  # the wallpaper of Sway itself is loaded by then
shot plain
nexa-living >/tmp/living.log 2>&1 &
LP=$!
sleep 2
shot desk1; sleep 0.5; shot desk2
[ "$(diff_in plain desk1 0 0 1366 768)" -gt 20 ] && pass "it draws the desktop (wallpaper + Nexa Signature)" || fail "draws" "$(cat /tmp/living.log)"
[ "$(diff_in desk1 desk2 0 0 1366 768)" -gt 0 ] && pass "the desktop moves" || fail "desktop still"
grim /out/1-desktop.png

(cd /src/settings-app && PYTHONPATH=/src/settings-app:/src/common python3 -m nexa_settings >/tmp/settings.log 2>&1 &)
for i in $(seq 40); do R=$(rect_of nexa-settings); [ -n "$R" ] && break; sleep 0.5; done
sleep 3
set -- $R; SX=$1; SY=$2; SW=$3; SH=$4
[ -n "$R" ] && pass "Settings is open ($SW x $SH at $SX,$SY)" || fail "Settings did not open" "$(tail -5 /tmp/settings.log)"
sleep 14  # past the start: the quality settles where frames really cost
[ "$(stat_of styled)" -ge 1 ] && pass "its window is found and styled (Holographic Grid)" || fail "styled" "$(cat $XDG_RUNTIME_DIR/nexa-living.stats)"
shot s1; sleep 0.4; shot s2
grim /out/2-settings.png
# a strip down the left of the window body: little text there
d=$(diff_in s1 s2 $((SX + 4)) $((SY + 60)) 40 $((SH - 80)))
[ "$d" -gt 0 ] && pass "its background moves behind the glass ($d)" || fail "settings background" "$d"
q=$(stat_of quality); fps=$(stat_of fps); ms=$(stat_of frame_ms); rss=$(stat_of rss_kb)
measure "quality=$q fps=$fps frame_ms=$ms (base $(stat_of base_ms), desktop $(stat_of desktop_ms), windows $(stat_of windows_ms)) rss_kb=$rss (Settings focused + desktop, 1366x768, pixman)"
t0=$(awk "{print \$14+\$15}" /proc/$LP/stat); sleep 10; t1=$(awk "{print \$14+\$15}" /proc/$LP/stat)
cpu=$(( (t1 - t0) * 100 / (10 * $(getconf CLK_TCK)) ))
measure "cpu=${cpu}% of one core over 10 s (balanced)"
[ "$cpu" -le 60 ] && pass "Balanced stays under 60% of one core here ($cpu%)" || fail "cpu" "$cpu%"

# Windows moved by hand (no compositor event): the background follows.
# Static, so frames compare exactly: where the window was must look like
# bare desktop again, the same as when the window is far away.
mg "magic.set_living(quality=\"static\")"; sleep 6
swaymsg "[app_id=nexa-settings] move position 300 100" >/dev/null; sleep 1.5
shot mv; grim /out/2b-moved.png
swaymsg "[app_id=nexa-settings] move position 500 120" >/dev/null; sleep 1.5
shot mvref
d=$(diff_in mv mvref 255 64 40 30)
[ "$d" -le 2 ] && pass "a moved window takes its background with it (nothing left behind)" || fail "moved" "$d"
mg "magic.set_living(quality=\"auto\")"; sleep 6
R=$(rect_of nexa-settings); set -- $R; SX=$1; SY=$2; SW=$3; SH=$4

# A second app focused: Settings holds still.
(cd /src/calculator && PYTHONPATH=/src/calculator:/src/common python3 -m nexa_calc >/tmp/calc.log 2>&1 &)
for i in $(seq 30); do C=$(rect_of nexa-calc); [ -n "$C" ] && break; sleep 0.5; done
sleep 3
set -- $C; CX=$1; CY=$2; CW=$3; CH=$4
swaymsg "[app_id=nexa-settings] move position 0 0" >/dev/null; swaymsg "[app_id=nexa-calc] move position 900 300; [app_id=nexa-calc] focus" >/dev/null
sleep 2; R=$(rect_of nexa-settings); set -- $R; SX=$1; SY=$2; SW=$3; SH=$4
shot u1; sleep 0.5; shot u2
grim /out/3-two-apps.png
# (Settings may redraw its own content now and then: a trace, not motion.)
[ "$(diff_in u1 u2 $((SX + 4)) $((SY + 60)) 30 200)" -le 2 ] && pass "unfocused, Settings holds still (no cost)" || fail "unfocused moved" "$(diff_in u1 u2 $((SX + 4)) $((SY + 60)) 30 200)"

# An app tells it about a click: a ripple.
# (the calculator style moves too: compare against Static)
mg "magic.set_living(quality=\"static\")"; sleep 7
shot r1; mg "magic.click(\"nexa-calc\", 60, 200)"; sleep 0.25; shot r2
grim /out/3b-ripple.png
[ "$(diff_in r1 r2 900 300 $CW $CH)" -gt 0 ] && pass "a click sent by the app ripples in its window" || fail "click"

# Static: drawn once, then nothing.
mg "magic.set_living(quality=\"static\")"; sleep 7
shot st1; sleep 1; shot st2
[ "$(stat_of quality)" = static ] && [ "$(diff_in st1 st2 0 0 1366 768)" = 0 ] && pass "Static: one picture, no frames" || fail "static" "$(stat_of quality) $(diff_in st1 st2 0 0 1366 768)"
t0=$(awk "{print \$14+\$15}" /proc/$LP/stat); sleep 5; t1=$(awk "{print \$14+\$15}" /proc/$LP/stat)
measure "cpu=$(( (t1 - t0) * 100 / (5 * $(getconf CLK_TCK)) ))% static"

# The theme: new colours, also in the backgrounds.
python3 -c "from nexa_common import themes; themes.apply(\"aurora\", reload=False)" >/dev/null 2>&1 || python3 -c "from nexa_common import themes; print(list(themes.THEMES))"
sleep 2; shot th1
grim /out/4-theme.png
[ "$(diff_in st2 th1 $((SX + 4)) $((SY + 60)) 60 $((SH - 80)))" -gt 3 ] && pass "a theme change recolours the backgrounds" || fail "theme" "$(diff_in st2 th1 $((SX + 4)) $((SY + 60)) 60 $((SH - 80)))"

# Low resource, then Off.
mg "magic.set_living(quality=\"low\")"; sleep 7
measure "low: fps=$(stat_of fps) frame_ms=$(stat_of frame_ms)"
mg "magic.set_living(quality=\"off\")"; sleep 6; shot off
grim /out/5-off.png
# top right, clear of both windows: the plain wallpaper again
[ "$(stat_of enabled)" = 0 ] && [ "$(diff_in plain off 1240 0 120 50)" -lt 25 ] && pass "Off: back to the plain wallpaper" || fail "off" "$(stat_of enabled) $(diff_in plain off 1240 0 120 50)"
mg "magic.reset(\"living\")"; sleep 2
kill -0 $LP && pass "still running after every change" || fail "nexa-living died" "$(tail -5 /tmp/living.log)"
grep -iE "error|warn" /tmp/living.log | head -3
exit $rc
'
