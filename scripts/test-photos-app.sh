#!/bin/sh
# Photos (apps/photos, C): unit tests, then the window in a headless Sway,
# started through nexa-run (confined to files.pictures, as when installed),
# then the package: built twice (byte-identical), verified, extracted and
# run from there. The Store's screenshots (apps/photos/shots) come from the
# window test. Screenshots: OUT_DIR.
# Usage: sh scripts/test-photos-app.sh [OUT_DIR]   (default dist/photos-test)
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
out=${1:-$repo/dist/photos-test}
mkdir -p "$out" "$repo/build/test"
out=$(cd "$out" && pwd)
[ -x "$repo/build/native/nexa-pack" ] && [ -x "$repo/build/native/nexa-run" ] || sh "$repo/scripts/build-native.sh" >/dev/null

echo "== build and unit tests"
docker run --rm -v "$repo:/src" -e OWNER="$(id -u):$(id -g)" "nexa-rust-apps:${version%.*}" sh -c '
set -e
cd /src/apps/photos
make -s test | grep -v "^PASS"
make -s build/photos
install -m 0755 build/photos /src/build/test/photos
rm -rf build
chown "$OWNER" /src/build/test/photos
'

# The window, installed and confined. $1: the program to run.
run_window() {
    docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$out:/out" -e OUT=/out -e PROGRAM="$1" -e SMOKE="${2:-}" \
        "nexa-apptest:${version%.*}" sh -c '
set -e
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 HOME=/root
mkdir -p $XDG_RUNTIME_DIR /tmp/bin /tmp/reg /tmp/libexec /tmp/lib && chmod 700 $XDG_RUNTIME_DIR
cp /src/build/native/nexa-run /tmp/bin/nexa-run
cp "$PROGRAM" /tmp/libexec/photos
echo "{\"id\": \"photos\", \"permissions\": [\"files.pictures\"]}" > /tmp/reg/photos.json
export PATH=/tmp/bin:$PATH NEXA_RUN_REGISTRY=/tmp/reg NEXA_RUN_LIBEXEC=/tmp/libexec NEXA_RUN_LIB=/tmp/lib
python3 -c "import sys; sys.path.insert(0, \"/src/common\"); from nexa_common import themes; themes.apply(\"horizon\", reload=False)" >/dev/null 2>&1 || true
printf "output * resolution 1280x800\ndefault_border none\nseat * xcursor_theme Adwaita 24\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1)
if [ -n "$SMOKE" ]; then
    python3 /src/apps/photos/tests/smoke.py
else
    python3 /src/apps/photos/tests/headless.py
fi
'
}

echo "== the window (through nexa-run)"
run_window /src/build/test/photos

echo "== the Store's screenshots"
mkdir -p "$repo/apps/photos/shots"
cp "$out/photos-library.png" "$repo/apps/photos/shots/library.png"
cp "$out/photos-viewer.png" "$repo/apps/photos/shots/viewer.png"
cp "$out/photos-albums.png" "$repo/apps/photos/shots/albums.png"
ls -l "$repo/apps/photos/shots"

echo "== the package"
docker run --rm -v "$repo:/src" -e OWNER="$(id -u):$(id -g)" "nexa-rust-apps:${version%.*}" sh -c '
set -e
cd /src/apps/photos
rm -rf build dist
/src/build/native/nexa-pack build | tail -1
a=$(sha256sum dist/photos-*.nexa | cut -d" " -f1)
mv dist/photos-1.0.0.nexa /tmp/first.nexa
rm -rf build
/src/build/native/nexa-pack build | tail -1
b=$(sha256sum dist/photos-*.nexa | cut -d" " -f1)
[ "$a" = "$b" ] && echo "PASS built twice: byte-identical ($a)" || { echo "FAIL two builds differ"; exit 1; }
/src/build/native/nexa-pack verify dist/photos-1.0.0.nexa --keys /src/assets/keys --json
rm -rf /tmp/x
/src/build/native/nexa-pack extract dist/photos-1.0.0.nexa /tmp/x --keys /src/assets/keys >/dev/null
ls /tmp/x /tmp/x/bin
install -m 0755 /tmp/x/bin/photos /src/build/test/photos-from-package
echo "package size: $(stat -c %s dist/photos-1.0.0.nexa) bytes"
chown -R "$OWNER" build dist /src/build/test/photos-from-package
'

echo "== run from the package"
run_window /src/build/test/photos-from-package smoke
