#!/bin/sh
# Nexa Browser end to end in the real system image (rootfs/ as a container):
# blocking, HTTPS-only, tabs, find, zoom, bookmarks, permissions, links that
# open new tabs (a pretend mouse), video (all formats), session restore.
# Screenshots: dist/browser-test. Needs rootfs/ (make refresh) and make native.
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
for b in build/native/nexa-webfilter build/test/nexa-vpointer; do
    [ -x "$repo/$b" ] || { echo "build it first: make native ($b)"; exit 1; }
done
sh "$repo/scripts/test-browser-media.sh" >/dev/null 2>&1 || true   # makes the clips and the image
mkdir -p "$repo/dist/browser-test"
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$repo/dist/media-test:/media:ro" \
    -v "$repo/dist/browser-test:/out" nexa-sys:test sh /src/common/tests/browser_e2e.sh
