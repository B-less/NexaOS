#!/bin/sh
# nexa-locker (C) in a headless Sway, inside the nexa-apptest container:
# it locks, a wrong password keeps it locked, the right one unlocks, a
# second lock screen is refused while locked, and if the lock screen dies
# the session stays locked (swaylock takes over). Typing goes through the
# compositor (wtype, a virtual keyboard), as real keys would. Enter goes in
# the typed text: wtype -k Return alone never reached any client here.
# Usage: sh scripts/test-locker.sh [OUT_DIR]   (screenshots; default dist/locker-test)
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-apptest:${version%.*}
docker image inspect "$image" >/dev/null 2>&1 || { echo "build $image first: sh scripts/test-app-headless.sh"; exit 1; }
[ -x "$repo/build/native/nexa-locker" ] || { echo "build it first: make native"; exit 1; }

out=${1:-$repo/dist/locker-test}
mkdir -p "$out"
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$(cd "$out" && pwd):/out" "$image" sh -c '
missing=$(for p in wtype grim swaylock-effects linux-pam font-inter font-jetbrains-mono; do apk info -e $p >/dev/null || echo $p; done)
[ -z "$missing" ] || apk add --no-cache -q $missing >/dev/null || { echo "couldn'"'"'t install: $missing"; exit 1; }
cp /src/lockscreen/nexa-lock.pam /etc/pam.d/nexa-lock
echo "root:Nexa-test-1" | chpasswd >/dev/null
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 HOME=/root NEXA_LOCKER_LIVE=0
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
printf "output * resolution 1280x800\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
# Started by name, as nexa-lock does (BusyBox pgrep -x matches argv[0]).
install -m 0755 /src/build/native/nexa-locker /usr/local/bin/nexa-locker
L=nexa-locker
rc=0
pass() { echo "PASS $1"; }
fail() { echo "FAIL $1"; rc=1; }

start=$(date +%s)
$L --daemonize && pass "locks and returns (--daemonize)" || fail "--daemonize didn'"'"'t lock"
pgrep -x nexa-locker >/dev/null && pass "the lock screen keeps running" || fail "no lock screen process"
sleep 1.5
grim /out/locked.png 2>/dev/null || true
# At rest it redraws once a second: a few percent of one CPU at most.
p=$(pgrep -n -x nexa-locker); t0=$(awk "{print \$14+\$15}" /proc/$p/stat); sleep 5; t1=$(awk "{print \$14+\$15}" /proc/$p/stat)
cpu=$(( (t1 - t0) * 100 / (5 * $(getconf CLK_TCK)) ))
[ $cpu -le 10 ] && pass "idle CPU use ${cpu}% of one core" || fail "idle CPU use ${cpu}% of one core"
timeout 5 $L; [ $? -eq 2 ] && pass "a second lock screen is refused while locked" || fail "a second lock screen was not refused"

printf "not-the-password\n" | wtype -
sleep 4
pgrep -x nexa-locker >/dev/null && pass "a wrong password keeps it locked" || fail "a wrong password unlocked it"
grim /out/wrong.png 2>/dev/null || true

printf "Nexa-test-1\n" | wtype -
for i in $(seq 40); do pgrep -x nexa-locker >/dev/null || break; sleep 0.2; done
pgrep -x nexa-locker >/dev/null && fail "the right password didn'"'"'t unlock" || pass "the right password unlocks"
timeout 5 $L --daemonize && pass "it can lock again after unlocking" || fail "couldn'"'"'t lock again"

# A crash while locked: the watchdog hands the lock to swaylock.
kill -SEGV $(pgrep -n -x nexa-locker)
sleep 3
pgrep -x swaylock >/dev/null && pass "after a crash swaylock takes over the lock" || fail "nothing took over after a crash"
timeout 5 $L; [ $? -eq 2 ] && pass "the session is still locked after the crash" || fail "the session was left unlocked"
grep -iE "error|assert" /tmp/sway.log | grep -v -iE "xwayland|dbus|backend|libinput|drm" | head -5
exit $rc
'
