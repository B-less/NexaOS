#!/bin/sh
# The Nexa Store window in a headless Sway (nexa-apptest container), against
# the real Rust engine (build/native/nexa-store) and real signed catalogues
# made with nexa-pack: a trusted one (the real Video Player plus test apps
# signed with a throwaway key made here) and an unsigned "Community" one. A
# stand-in broker records install/remove. Screenshots go to OUT_DIR.
# Usage: sh scripts/test-store-app.sh [OUT_DIR]   (default dist/store-test)
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$repo/dist/store-test}
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-apptest:${version%.*}
docker image inspect "$image" >/dev/null 2>&1 || { echo "build $image first: sh scripts/test-app-headless.sh"; exit 1; }
[ -x "$repo/build/native/nexa-store" ] || sh "$repo/scripts/build-native.sh"
[ -f "$repo/apps/video-player/dist/video-player-1.0.4.nexa" ] || sh "$repo/scripts/build-apps.sh"
mkdir -p "$out"

docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$(cd "$out" && pwd):/out" -e OUT=/out "$image" sh -c '
set -e
P=/src/build/native/nexa-pack
W=/tmp/store; mkdir -p $W/keys $W/trusted/packages $W/community/packages $W/registry $W/home
cp /src/assets/keys/*.pub $W/keys/
# A throwaway key, trusted the way NexaOS trusts keys: keys/<key id>.pub.
cd $W && kid=$($P keygen tester | sed -n "s/^key id //p") && mv tester.pub keys/$kid.pub && cd - >/dev/null
mkapp() { # id name version categories perms catalogue signed description
    d=$W/src/$1; mkdir -p $d/build; printf "#!/bin/sh\necho %s\n" "$1" > $d/build/$1; chmod +x $d/build/$1
    cp /src/assets/icons/$9 $d/icon.svg
    printf "[package]\nid = \"%s\"\nname = \"%s\"\nversion = \"%s\"\ndescription = \"%s\"\ncategories = \"%s\"\nicon = \"icon.svg\"\n\n[build]\ncommand = \"true\"\nbinary = \"build/%s\"\n\n[requires]\ndepends = []\npermissions = [%s]\n" \
        "$1" "$2" "$3" "$8" "$4" "$1" "$5" > $d/nexa.toml
    key=""; [ "$7" = signed ] && key="--key $W/tester.key"
    (cd $d && $P build $key >/dev/null) && cp $d/dist/*.nexa $W/$6/packages/
}
mkapp notes-pro "Notes Pro" 1.2.0 "Office;" "\"files.documents\"" trusted signed "Notes with folders, tags and quick search" nexa-notes.svg
mkapp sketch "Sketch" 2.1.0 "Graphics;" "\"files.pictures\"" trusted signed "Draw and paint with layers and brushes" nexa-colorpicker.svg
mkapp chat "Chat" 0.9.1 "Network;" "\"network\", \"microphone\", \"notifications\"" trusted signed "Talk with friends: messages and voice" nexa-browser.svg
mkapp clock "Clock" 1.0.0 "Utility;" "" trusted signed "Alarms, timers and a stopwatch" nexa-power.svg
mkapp hexes "Hexes" 0.3.0 "Game;" "" community unsigned "A puzzle of sliding hexagons" nexa-nexus.svg
cp /src/apps/video-player/dist/video-player-1.0.4.nexa $W/trusted/packages/
$P repo build $W/trusted --key $W/tester.key >/dev/null
$P repo build $W/community >/dev/null
printf "{\"catalogues\": [{\"name\": \"NexaOS\", \"url\": \"file://$W/trusted\"}, {\"name\": \"Community\", \"url\": \"file://$W/community\"}, {\"name\": \"Online\", \"url\": \"https://nexa-apps.invalid\"}]}" > $W/store.conf
# Installed already: Notes Pro 1.0.0 (an update waits) and Clock.
printf "{\"id\":\"notes-pro\",\"version\":\"1.0.0\",\"trust\":\"trusted\",\"permissions\":[\"files.documents\"],\"paths\":[]}" > $W/registry/notes-pro.json
printf "{\"id\":\"clock\",\"version\":\"1.0.0\",\"trust\":\"trusted\",\"permissions\":[],\"paths\":[]}" > $W/registry/clock.json

export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 HOME=$W/home
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
printf "output * resolution 1280x800 bg /src/assets/wallpaper.png fill\ndefault_border none\nfor_window [app_id=\".*\"] floating enable\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1)
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
PYTHONPATH=/src/common python3 -c "from nexa_common import themes; themes.apply(reload=False)" >/dev/null 2>&1 || true
export NEXA_STORE_BIN=/src/build/native/nexa-store NEXA_STORE_CONF=$W/store.conf NEXA_STORE_KEYS=$W/keys NEXA_STORE_REGISTRY=$W/registry
export NEXA_BROKER_SOCKET=$W/broker.sock STORE_TEST_DIR=$W
set +e
python3 /src/store/tests/headless_store.py 2>/tmp/err.log; rc=$?
grep -E "Traceback|Error|CRITICAL" -A4 /tmp/err.log | grep -v "xdg-toplevel-icon" | head -30
exit $rc
'
