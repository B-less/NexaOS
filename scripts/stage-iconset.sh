#!/bin/sh
# Stage the NexaHexa icons (assets/iconset, made by `make iconset`) into the
# rootfs: the base set (app tiles + symbolic icons) and a set per theme and
# icon style (NexaHexa-<theme>, -<theme>-dark, -<theme>-tinted). The theme
# picks one at login and on every change (themes.icon_theme_for); their
# caches are built with the others by build-disk-image.sh.
set -eu

rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}
src="$base/assets/iconset"

[ -f "$src/NexaHexa/index.theme" ] || { echo "no NexaHexa icons (make iconset): not staged"; exit 0; }
mkdir -p "$rootfs/usr/share/icons"
# Sets that are no longer made (a theme removed) go too.
rm -rf "$rootfs"/usr/share/icons/NexaHexa "$rootfs"/usr/share/icons/NexaHexa-*
n=0
for set in "$src"/NexaHexa "$src"/NexaHexa-*; do
    [ -f "$set/index.theme" ] || continue
    cp -a "$set" "$rootfs/usr/share/icons/"
    n=$((n + 1))
done
chmod -R a+rX "$rootfs"/usr/share/icons/NexaHexa*
echo "NexaHexa: $n icon sets staged into $rootfs/usr/share/icons"
