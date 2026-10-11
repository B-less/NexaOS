#!/bin/sh
# Rasterizes assets/icons/*.svg to PNG at build time (rsvg-convert is only
# needed here, in the build container — the shipped OS gets plain PNGs).
set -eu

SRC_DIR=$1
OUT_DIR=$2

apk add --no-cache rsvg-convert >/dev/null
mkdir -p "$OUT_DIR"
count=0
for svg in "$SRC_DIR"/*.svg; do
    name=$(basename "$svg" .svg)
    rsvg-convert -w 128 -h 128 "$svg" -o "$OUT_DIR/$name.png"
    count=$((count + 1))
done
echo "[OK] icons rasterized: $count files"
