#!/bin/sh
# Orbit (tools-c/nexa-orbit): unit tests, then the rail in a headless Sway
# with a real pointer (nexa-vpointer). Screenshots: OUT_DIR.
# Usage: sh scripts/test-orbit.sh [OUT_DIR]   (default dist/orbit-test)
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
out=${1:-$repo/dist/orbit-test}
mkdir -p "$out" "$repo/build/test"
docker run --rm -v "$repo:/src" -e OWNER="$(id -u):$(id -g)" "nexa-rust-apps:${version%.*}" sh -c '
set -e
cd /src/tools-c/nexa-orbit
make -s test
make -s nexa-orbit
install -m 0755 nexa-orbit /src/build/test/nexa-orbit
rm -f nexa-orbit orbit-test
if [ ! -x /src/build/test/nexa-vpointer ] || [ /src/common/tests/c/vpointer.c -nt /src/build/test/nexa-vpointer ]; then
    mkdir -p /tmp/vp
    wayland-scanner client-header /usr/share/wlr-protocols/unstable/wlr-virtual-pointer-unstable-v1.xml /tmp/vp/virtual-pointer.h
    wayland-scanner private-code /usr/share/wlr-protocols/unstable/wlr-virtual-pointer-unstable-v1.xml /tmp/vp/virtual-pointer.c
    cc -O2 -Wall -Wextra -Wno-unused-parameter -I/tmp/vp -o /src/build/test/nexa-vpointer /src/common/tests/c/vpointer.c /tmp/vp/virtual-pointer.c $(pkg-config --cflags --libs wayland-client)
fi
chown "$OWNER" /src/build/test/nexa-orbit /src/build/test/nexa-vpointer
'
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$(cd "$out" && pwd):/out" -e OUT=/out "nexa-apptest:${version%.*}" sh -c '
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 HOME=/root
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
printf "output * resolution 1280x800 bg /src/assets/wallpaper.png fill\nseat * xcursor_theme Adwaita 24\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1) WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
PYTHONPATH=/src/common python3 -c "from nexa_common import themes; themes.apply(\"horizon\", reload=False)" >/dev/null 2>&1
cp /src/build/test/nexa-orbit /tmp/nexa-orbit
python3 /src/tools-c/nexa-orbit/tests/headless_orbit.py
'
