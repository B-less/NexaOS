#!/bin/sh
# Nexa Link's screens (Settings > Nexa Link, the pop-ups, Files' Send to a
# Device) in a headless Sway, with a real second device: a "phone"
# container running nexa-linkd on the same private Docker network.
# See common/tests/headless_link_ui.py. Screenshots: dist/link-ui-test.
set -u
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-apptest:${version%.*}
bin="$repo/build/native/nexa-linkd"
[ -x "$bin" ] || { echo "build it first: make native"; exit 1; }
docker image inspect "$image" >/dev/null 2>&1 || { echo "build $image first: sh scripts/test-app-headless.sh"; exit 1; }
out=${1:-$repo/dist/link-ui-test}
mkdir -p "$out"
net=nexa-link-ui-test
shared=$(mktemp -d)
chmod 777 "$shared"
cleanup() { docker rm -f nlu-phone >/dev/null 2>&1; docker network rm "$net" >/dev/null 2>&1; rm -rf "$shared"; }
cleanup; shared=$(mktemp -d); chmod 777 "$shared"
trap cleanup EXIT
docker network create "$net" >/dev/null || exit 1
# The phone: its own Nexa Link service, and a little loop that runs the
# commands the test writes to the shared folder.
docker run -d --name nlu-phone --hostname phone --network "$net" -v "$bin:/usr/local/bin/nexa-linkd:ro" -v "$shared:/shared" \
    -e XDG_RUNTIME_DIR=/tmp/run -e HOME=/root "alpine:${version%.*}" sh -c '
mkdir -p /tmp/run && chmod 700 /tmp/run
nexa-linkd name "OnePlus 6"
nexa-linkd serve >/tmp/serve.log 2>&1 &
while true; do
    if [ -f /shared/phone.cmd ]; then
        mv /shared/phone.cmd /tmp/cmd
        sh /tmp/cmd > /shared/phone.tmp 2>&1
        mv /shared/phone.tmp /shared/phone.out
    fi
    sleep 0.2
done' >/dev/null || exit 1
docker run --rm --cap-add SYS_NICE --network "$net" -v "$repo:/src:ro" -v "$(cd "$out" && pwd):/out" -v "$shared:/shared" \
    -v "$bin:/usr/local/bin/nexa-linkd:ro" -e HOME=/root -e OUT=/out -e PHONE_DIR=/shared "$image" sh -c '
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
export DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address)
printf "output * resolution 1280x800 bg /src/assets/wallpaper.png fill\nfor_window [app_id=\".*\"] floating enable\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1)
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
PYTHONPATH=/src/common python3 -c "from nexa_common import themes; themes.apply(reload=False)" >/dev/null 2>&1
nexa-linkd name "HP EliteBook"
nexa-linkd serve >/tmp/serve.log 2>&1 &
sleep 1
python3 /src/common/tests/headless_link_ui.py 2>/tmp/err.log; rc=$?
grep -E "Traceback|Error" -A6 /tmp/err.log | grep -v "xdg-toplevel-icon" | head -30
exit $rc
'
