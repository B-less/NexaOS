#!/bin/sh
# Rasterizes the NexaOS wallpaper SVG to PNG at build time. rsvg-convert and
# fonts are only needed here (in the build container) — the shipped OS just
# gets a plain PNG, so swaybg doesn't need SVG support at runtime.
set -eu

SRC=$1
OUT=$2

apk add --no-cache rsvg-convert font-noto ttf-dejavu fontconfig >/dev/null
fc-cache -f >/dev/null 2>&1 || true
mkdir -p "$(dirname "$OUT")"
rsvg-convert -w 1920 -h 1080 "$SRC" -o "$OUT"
echo "[OK] wallpaper: $(du -h "$OUT" | cut -f1)"
