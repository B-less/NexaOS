#!/bin/sh
# Run one NexaOS GTK app in a headless Sway inside an Alpine container and
# save a screenshot, to check layout without a VM or real GPU.
#
# Usage: sh scripts/test-app-headless.sh APP_DIR MODULE [OUT.png] [SWAY_CMD]
#   e.g. sh scripts/test-app-headless.sh pulse nexa_pulse dist/pulse.png
# SWAY_CMD (optional) is a swaymsg command run just before the screenshot.
# Env: APP_ARGS (arguments for the app), WAIT (seconds before the shot, 5),
#      SETUP (shell commands run in the container before the app starts;
#      it can write "export VAR=..." lines to /tmp/app.env for the app).
# WebKit's bubblewrap sandbox can't nest inside Docker and there is no GPU, so the
# sandbox and GPU rendering are off here;
# the sandbox itself is checked on the real image (scripts/test-live-boot.py).
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
app=$1
module=$2
out=${3:-$repo/dist/$module.png}
sway_cmd=${4:-}
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-apptest:${version%.*}
# Built once and reused: installing GTK + WebKit takes minutes.
if ! docker image inspect "$image" >/dev/null 2>&1; then
    printf 'FROM alpine:%s\nRUN apk add -q sway grim py3-gobject3 py3-cairo gtk+3.0 gtk-layer-shell webkit2gtk-4.1 tor curl font-dejavu adwaita-icon-theme hicolor-icon-theme\n' \
        "${version%.*}" | docker build -q -t "$image" - >/dev/null
fi
mkdir -p "$(dirname "$out")"
outdir=$(cd "$(dirname "$out")" && pwd)

docker run --rm --cap-add SYS_NICE \
    -v "$repo:/src:ro" -v "$outdir:/out" \
    -e APP="$app" -e MODULE="$module" -e SHOT="$(basename "$out")" -e SWAY_CMD="$sway_cmd" \
    -e APP_ARGS="${APP_ARGS:-}" -e WAIT="${WAIT:-5}" -e SETUP="${SETUP:-}" -e HOME=/root \
    -e WEBKIT_DISABLE_SANDBOX=1 -e WEBKIT_DISABLE_DMABUF_RENDERER=1 -e WEBKIT_DISABLE_COMPOSITING_MODE=1 \
    "$image" sh -c '
set -e
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
printf "output * resolution 1280x800 bg #1a1f2e solid_color\ndefault_border none\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1)
[ -n "$SETUP" ] && sh -c "$SETUP"
[ -f /tmp/app.env ] && . /tmp/app.env  # SETUP can export variables for the app here
PYTHONPATH=/src/$APP:/src/common python3 -m $MODULE $APP_ARGS >/tmp/app.log 2>&1 &
sleep ${WAIT:-5}
[ -n "$SWAY_CMD" ] && swaymsg "$SWAY_CMD" >/dev/null && sleep 1
grim /out/$SHOT
chmod 666 /out/$SHOT
echo "--- app log"; cat /tmp/app.log
'
echo "screenshot: $out"
