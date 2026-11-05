#!/bin/sh
# Deck (tools-c/nexa-deck): unit tests, then Deck in a headless Sway against
# fake iwd, BlueZ, MPRIS, broker, wpctl and nexa-signalctl, clicked with a real
# pointer. Screenshots: OUT_DIR.
# Usage: sh scripts/test-deck.sh [OUT_DIR]   (default dist/deck-test)
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
out=${1:-$repo/dist/deck-test}
mkdir -p "$out" "$repo/build/test"
docker run --rm -v "$repo:/src" -e OWNER="$(id -u):$(id -g)" "nexa-rust-apps:${version%.*}" sh -c '
set -e
cd /src/tools-c/nexa-deck
make -s test | tail -1
make -s nexa-deck
install -m 0755 nexa-deck /src/build/test/nexa-deck
rm -f nexa-deck deck-test
if [ ! -x /src/build/test/nexa-vpointer ] || [ /src/common/tests/c/vpointer.c -nt /src/build/test/nexa-vpointer ]; then
    mkdir -p /tmp/vp
    wayland-scanner client-header /usr/share/wlr-protocols/unstable/wlr-virtual-pointer-unstable-v1.xml /tmp/vp/virtual-pointer.h
    wayland-scanner private-code /usr/share/wlr-protocols/unstable/wlr-virtual-pointer-unstable-v1.xml /tmp/vp/virtual-pointer.c
    cc -O2 -Wall -Wextra -Wno-unused-parameter -I/tmp/vp -o /src/build/test/nexa-vpointer /src/common/tests/c/vpointer.c /tmp/vp/virtual-pointer.c $(pkg-config --cflags --libs wayland-client)
fi
chown "$OWNER" /src/build/test/nexa-deck /src/build/test/nexa-vpointer
'
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$(cd "$out" && pwd):/out" -e OUT=/out -e NO_AT_BRIDGE="${NO_AT_BRIDGE:-}" -e DECK_DEBUG="${DECK_DEBUG:-}" "nexa-apptest:${version%.*}" sh -c '
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 HOME=/root
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
T=/src/tools-c/nexa-deck/tests
# the fakes: a private bus standing in for both buses, sysfs, tools
DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address | head -1)
export DBUS_SESSION_BUS_ADDRESS DBUS_SYSTEM_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS
python3 /src/common/tests/mock_iwd.py >/dev/null 2>&1 &
python3 /src/common/tests/mock_bluez.py >/dev/null 2>&1 &
mkdir -p /tmp/bl/intel_backlight /tmp/ps/BAT0
echo 500 > /tmp/bl/intel_backlight/brightness; echo 1000 > /tmp/bl/intel_backlight/max_brightness
echo 64 > /tmp/ps/BAT0/capacity; echo Discharging > /tmp/ps/BAT0/status
echo 30000000 > /tmp/ps/BAT0/energy_now; echo 50000000 > /tmp/ps/BAT0/energy_full; echo 12000000 > /tmp/ps/BAT0/power_now
echo "45 0" > /tmp/volume; echo default > /tmp/modes
cat > /tmp/history.json << "JSON"
{"type": "aa{sv}", "data": [[{"app-name": {"type": "s", "data": "Store"}, "summary": {"type": "s", "data": "Update ready"}, "body": {"type": "s", "data": "Files 3.4 can be installed"}, "id": {"type": "u", "data": 12}}, {"app-name": {"type": "s", "data": "Files"}, "summary": {"type": "s", "data": "Copied 3 files"}, "body": {"type": "s", "data": ""}, "id": {"type": "u", "data": 9}}]]}
JSON
python3 $T/fakes.py all >/dev/null 2>&1 &
export NO_AT_BRIDGE=${NO_AT_BRIDGE:-} PATH=$T/bin:$PATH NEXA_BROKER_SOCKET=/tmp/broker.sock NEXA_DECK_BACKLIGHT=/tmp/bl NEXA_DECK_POWER_SUPPLY=/tmp/ps
printf "output * resolution 1280x800 bg /src/assets/wallpaper.png fill\nseat * xcursor_theme Adwaita 24\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1) WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
PYTHONPATH=/src/common python3 -c "from nexa_common import themes; themes.apply(\"horizon\", reload=False)" >/dev/null 2>&1
cp /src/build/test/nexa-deck /tmp/nexa-deck
sleep 0.5
python3 $T/headless_deck.py || { echo "--- nexa-deck said:"; tail -20 /tmp/deck.err; exit 1; }
'
