#!/bin/sh
# Strata (tools-c/nexa-strata): unit tests, then the layers in a headless
# Sway with Orbit (the stack), Living (the tint) and a real pointer.
# Screenshots: OUT_DIR.
# Usage: sh scripts/test-strata.sh [OUT_DIR]   (default dist/strata-test)
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
out=${1:-$repo/dist/strata-test}
mkdir -p "$out" "$repo/build/test"
docker run --rm -v "$repo:/src" -e OWNER="$(id -u):$(id -g)" "nexa-rust-apps:${version%.*}" sh -c '
set -e
cd /src/tools-c/nexa-strata
make -s test
make -s nexa-strata
rm -f strata-test
for t in nexa-strata nexa-orbit nexa-living; do
    make -s -C /src/tools-c/$t $t
    install -m 0755 /src/tools-c/$t/$t /src/build/test/$t
    rm -f /src/tools-c/$t/$t
done
if [ ! -x /src/build/test/nexa-vpointer ] || [ /src/common/tests/c/vpointer.c -nt /src/build/test/nexa-vpointer ]; then
    mkdir -p /tmp/vp
    wayland-scanner client-header /usr/share/wlr-protocols/unstable/wlr-virtual-pointer-unstable-v1.xml /tmp/vp/virtual-pointer.h
    wayland-scanner private-code /usr/share/wlr-protocols/unstable/wlr-virtual-pointer-unstable-v1.xml /tmp/vp/virtual-pointer.c
    cc -O2 -Wall -Wextra -Wno-unused-parameter -I/tmp/vp -o /src/build/test/nexa-vpointer /src/common/tests/c/vpointer.c /tmp/vp/virtual-pointer.c $(pkg-config --cflags --libs wayland-client)
fi
chown "$OWNER" /src/build/test/nexa-strata /src/build/test/nexa-orbit /src/build/test/nexa-living /src/build/test/nexa-vpointer
'
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$(cd "$out" && pwd):/out" -e OUT=/out "nexa-apptest:${version%.*}" sh -c '
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 HOME=/root GSETTINGS_BACKEND=keyfile
mkdir -p $XDG_RUNTIME_DIR /tmp/bin && chmod 700 $XDG_RUNTIME_DIR
for t in nexa-strata nexa-orbit nexa-living; do cp /src/build/test/$t /tmp/bin/$t; done
export PATH=/tmp/bin:$PATH
# The session'"'"'s own Strata bindings, so Sway has to accept them.
{
    printf "set \$mod Mod4\noutput * resolution 1280x800\nseat * xcursor_theme Adwaita 24\n"
    printf "for_window [app_id=\".*\"] floating enable\n"
    sed -n "/^bindsym.*nexa-strata/p" /src/scripts/generate-configs.sh
} > /tmp/sway.conf
# (its own runtime folder: the check leaves a Wayland socket behind)
mkdir -p /tmp/xdg-check && chmod 700 /tmp/xdg-check
XDG_RUNTIME_DIR=/tmp/xdg-check sway -C -c /tmp/sway.conf > /tmp/sway-validate.log 2>&1; echo "$?" > /tmp/sway-validate.rc
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1) WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
PYTHONPATH=/src/common python3 -c "from nexa_common import themes; themes.apply(\"horizon\", reload=False)" >/dev/null 2>&1
python3 /src/tools-c/nexa-strata/tests/headless_strata.py
'
