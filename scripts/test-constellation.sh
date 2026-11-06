#!/bin/sh
# Constellation (tools-c/nexa-constellation, tools/nexa-starmap): unit tests
# (C and Rust), then the sky in a headless Sway inside Orbit, opened with the
# session's own Super+Tab binding, driven with real keys (wtype) and a real
# pointer (nexa-vpointer). Screenshots: OUT_DIR.
# Usage: sh scripts/test-constellation.sh [OUT_DIR]   (default dist/constellation-test)
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
out=${1:-$repo/dist/constellation-test}
mkdir -p "$out" "$repo/build/test" /var/tmp/nexa-cargo
[ -x "$repo/build/test/nexa-vpointer" ] || sh "$repo/scripts/build-c-tools.sh" >/dev/null

echo "== C: the chart"
docker run --rm -v "$repo:/src" -e OWNER="$(id -u):$(id -g)" "nexa-rust-apps:${version%.*}" sh -c '
set -e
cd /src/tools-c/nexa-constellation
make -s test | grep -v "^PASS"
rm -f /src/tools-c/nexa-constellation/chart-test
for t in nexa-orbit nexa-constellation; do
    make -s -C /src/tools-c/$t $t
    install -m 0755 /src/tools-c/$t/$t /src/build/test/$t
done
rm -f /src/tools-c/nexa-orbit/nexa-orbit /src/tools-c/nexa-constellation/nexa-constellation
chown "$OWNER" /src/build/test/nexa-orbit /src/build/test/nexa-constellation
'
echo "== Rust: nexa-starmap"
docker run --rm -v "$repo:/src" -v /var/tmp/nexa-cargo:/root/.cargo/registry -e OWNER="$(id -u):$(id -g)" -w /src/tools "nexa-rust:${version%.*}" sh -c '
set -e
cargo test -q -p nexa-starmap 2>&1 | grep -E "^test result: ok. [1-9]|FAILED|panicked" || { cargo test -q -p nexa-starmap 2>&1 | tail -30; exit 1; }
target=$(rustc -vV | sed -n "s/^host: //p")
RUSTFLAGS="-C target-feature=+crt-static" cargo build --release -q -p nexa-starmap --target $target
install -m 0755 target/$target/release/nexa-starmap /src/build/test/
chown -R "$OWNER" /src/tools/target /src/tools/Cargo.lock /src/build/test/nexa-starmap
'
if ! docker run --rm "nexa-skytest:${version%.*}" sh -c 'command -v wtype' >/dev/null 2>&1; then
    printf 'FROM nexa-apptest:%s\nRUN apk add -q wtype\n' "${version%.*}" | docker build -q -t "nexa-skytest:${version%.*}" - >/dev/null
fi
echo "== the sky"
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$(cd "$out" && pwd):/out" -e OUT=/out "nexa-skytest:${version%.*}" sh -c '
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 HOME=/root GSETTINGS_BACKEND=keyfile
mkdir -p $XDG_RUNTIME_DIR /tmp/bin && chmod 700 $XDG_RUNTIME_DIR
for t in nexa-orbit nexa-constellation nexa-starmap; do cp /src/build/test/$t /tmp/bin/$t; done
export PATH=/tmp/bin:$PATH
# The session'"'"'s own bindings for Super+Tab and Alt+Tab, so Sway has to accept them.
{
    printf "set \$mod Mod4\noutput * resolution 1280x800\nseat * xcursor_theme Adwaita 24\n"
    printf "for_window [app_id=\".*\"] floating enable\n"
    sed -n "/^bindsym \$mod+Tab /p; /^bindsym Mod1+.*Tab /p" /src/scripts/generate-configs.sh
} > /tmp/sway.conf
mkdir -p /tmp/xdg-check && chmod 700 /tmp/xdg-check
XDG_RUNTIME_DIR=/tmp/xdg-check sway -C -c /tmp/sway.conf > /tmp/sway-validate.log 2>&1; echo "$?" > /tmp/sway-validate.rc
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1) WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
PYTHONPATH=/src/common python3 -c "from nexa_common import themes; themes.apply(\"horizon\", reload=False)" >/dev/null 2>&1
python3 /src/tools-c/nexa-constellation/tests/headless_constellation.py
'
