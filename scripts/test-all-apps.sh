#!/bin/sh
# Debug sweep: start every NexaOS app (and every Settings page) one by one
# in a headless Sway inside the test container, screenshot it, and report
# any Python traceback or GTK critical in its log.
#
# Usage: sh scripts/test-all-apps.sh [OUT_DIR]   (default dist/sweep)
# Uses the image built by scripts/test-app-headless.sh.
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$repo/dist/sweep}
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-apptest:${version%.*}
docker image inspect "$image" >/dev/null 2>&1 || { echo "build $image first: sh scripts/test-app-headless.sh"; exit 1; }
mkdir -p "$out"

docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$(cd "$out" && pwd):/out" \
    -e HOME=/root -e WEBKIT_DISABLE_SANDBOX=1 -e WEBKIT_DISABLE_DMABUF_RENDERER=1 \
    -e WEBKIT_DISABLE_COMPOSITING_MODE=1 "$image" sh -c '
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
printf "output * resolution 1280x800 bg /src/assets/wallpaper.png fill\ndefault_border none\nfor_window [app_id=\".*\"] floating enable\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1)
# What a real session has: icons, app entries, the theme files.
mkdir -p /usr/share/icons/hicolor/scalable/apps /usr/share/applications
cp /src/assets/icons/*.svg /usr/share/icons/hicolor/scalable/apps/
gtk-update-icon-cache -q /usr/share/icons/hicolor 2>/dev/null || true
for f in /src/scripts/stage-*.sh; do
    sed -n "/^cat > .*applications\/.*desktop.*<< .DESKTOP_EOF./,/^DESKTOP_EOF/p" "$f" | sed "1d;\$d" \
        | awk -v RS="\\\\[Desktop Entry\\\\]" "NR>1 {n++; print \"[Desktop Entry]\" \$0 > \"/usr/share/applications/sweep-\" FILENAME n \".desktop\"}" FILENAME=$(basename $f .sh) 2>/dev/null || true
done
PYTHONPATH=/src/common python3 -c "from nexa_common import themes; themes.apply(reload=False)" >/dev/null 2>&1
P=/src/common
run() {  # name dir module [args...]
    name=$1; dir=$2; mod=$3; shift 3
    swaymsg "[app_id=\".*\"] kill" >/dev/null 2>&1; sleep 0.5
    PYTHONPATH=/src/$dir:$P timeout 12 python3 -m $mod "$@" >/tmp/$name.log 2>&1 &
    sleep ${SWEEP_WAIT:-4}
    grim /out/$name.png 2>/dev/null
    bad=$(grep -E "Traceback|Error|CRITICAL|Gtk-WARNING|exception" /tmp/$name.log | grep -vE "Glycin|xdg-toplevel-icon|libEGL|MESA" | head -3)
    if [ -n "$bad" ]; then echo "PROBLEM $name: $bad"; cp /tmp/$name.log /out/$name.log; else echo "ok      $name"; fi
    pkill -f "python3 -m $mod" 2>/dev/null; sleep 0.3
}
run archive      archive         nexa_archive
SWEEP_WAIT=12 run browser browser nexa_browser about:blank  # WebKit takes ~10 s to start here
run calculator   calculator      nexa_calc
run chars        chars           nexa_chars
run clipboard    clipboard       nexa_clipboard
run colorpicker  colorpicker     nexa_colorpicker
run files        file-manager    nexa_files
run viewer       image-viewer    nexa_viewer /src/assets/wallpaper.png
run installer    installer-os    nexa_setup
run pkg-installer installer      nexa_installer
run music        music-player    nexa_music
run network      network-settings nexa_network
run assistant    nexa-ai         nexa_ai.gui
run notes        notes           nexa_notes
run pdf          pdf-viewer      nexa_pdf
run power        power           nexa_power
run pulse        pulse           nexa_pulse
run pulse-perf   pulse           nexa_pulse --performance
run software     software-center nexa_software
run editor       text-editor     nexa_edit
run volume       volume-mixer    nexa_volume
for page in personalization bluetooth vpn system time power assistant about; do
    run settings-$page settings-app nexa_settings $page
done
for panel in start control launchpad dock; do
    run shell-$panel shell nexa_shell $panel
done
'
echo "screenshots and failing logs: $out"
