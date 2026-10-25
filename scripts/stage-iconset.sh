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
# Apps from Alpine ask for their own icon names: point them at the NexaHexa
# tile, or they show up as the only square icons in the Dock and Nexus.
#   alias        NexaHexa icon
for pair in foot:nexa-terminal footclient:nexa-terminal org.codeberg.dnkl.foot:nexa-terminal; do
    alias=${pair%%:*} target=${pair#*:}
    for dir in "$rootfs"/usr/share/icons/NexaHexa*/scalable/apps; do
        [ -f "$dir/$target.svg" ] && ln -sf "$target.svg" "$dir/$alias.svg"
    done
done
chmod -R a+rX "$rootfs"/usr/share/icons/NexaHexa*
echo "NexaHexa: $n icon sets staged into $rootfs/usr/share/icons"
