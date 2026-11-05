#!/bin/sh
# Signals: nexa-signald's unit tests, then the service and its tabs in a
# headless Sway, notifications sent over D-Bus, clicked with a real pointer.
# Screenshots: OUT_DIR.
# Usage: sh scripts/test-signals.sh [OUT_DIR]   (default dist/signals-test)
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
out=${1:-$repo/dist/signals-test}
mkdir -p "$out" "$repo/build/test" /var/tmp/nexa-cargo
docker run --rm -v "$repo:/src" -v /var/tmp/nexa-cargo:/root/.cargo/registry -e OWNER="$(id -u):$(id -g)" -w /src/tools "nexa-rust:${version%.*}" sh -c '
set -e
cargo test -q -p nexa-signald 2>&1 | grep -E "^test result: ok. [1-9]|FAILED|panicked" || { cargo test -q -p nexa-signald 2>&1 | tail -30; exit 1; }
target=$(rustc -vV | sed -n "s/^host: //p")
RUSTFLAGS="-C target-feature=+crt-static" cargo build --release -q -p nexa-signald --target $target
install -m 0755 target/$target/release/nexa-signald target/$target/release/nexa-signalctl /src/build/test/
chown -R "$OWNER" /src/tools/target /src/tools/Cargo.lock /src/build/test
'
docker run --rm -v "$repo:/src" -e OWNER="$(id -u):$(id -g)" "nexa-rust-apps:${version%.*}" sh -c '
set -e
make -s -C /src/tools-c/nexa-signals nexa-signals
install -m 0755 /src/tools-c/nexa-signals/nexa-signals /src/build/test/nexa-signals
rm -f /src/tools-c/nexa-signals/nexa-signals
if [ ! -x /src/build/test/nexa-vpointer ] || [ /src/common/tests/c/vpointer.c -nt /src/build/test/nexa-vpointer ]; then
    mkdir -p /tmp/vp
    wayland-scanner client-header /usr/share/wlr-protocols/unstable/wlr-virtual-pointer-unstable-v1.xml /tmp/vp/virtual-pointer.h
    wayland-scanner private-code /usr/share/wlr-protocols/unstable/wlr-virtual-pointer-unstable-v1.xml /tmp/vp/virtual-pointer.c
    cc -O2 -Wall -Wextra -Wno-unused-parameter -I/tmp/vp -o /src/build/test/nexa-vpointer /src/common/tests/c/vpointer.c /tmp/vp/virtual-pointer.c $(pkg-config --cflags --libs wayland-client)
fi
chown "$OWNER" /src/build/test/nexa-signals /src/build/test/nexa-vpointer
'
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$(cd "$out" && pwd):/out" -e OUT=/out "nexa-apptest:${version%.*}" sh -c '
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 HOME=/root
mkdir -p $XDG_RUNTIME_DIR /tmp/sig && chmod 700 $XDG_RUNTIME_DIR
cp /src/build/test/nexa-signald /src/build/test/nexa-signalctl /src/build/test/nexa-signals /tmp/sig/
DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address | head -1); export DBUS_SESSION_BUS_ADDRESS
printf "output * resolution 1280x800 bg /src/assets/wallpaper.png fill\nseat * xcursor_theme Adwaita 24\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1) WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
until swaymsg -t get_outputs 2>/dev/null | grep -q "\"active\": true"; do sleep 0.2; done; sleep 0.5
PYTHONPATH=/src/common python3 -c "from nexa_common import themes; themes.apply(\"horizon\", reload=False)" >/dev/null 2>&1
python3 /src/tools-c/nexa-signals/tests/headless_signals.py || { echo "--- they said:"; tail -8 /tmp/signals.err /tmp/signald.err; exit 1; }
'
