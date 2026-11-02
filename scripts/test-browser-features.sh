#!/bin/sh
# Nexa Browser's pinned and muted tabs, tab list, bookmark manager (import
# and export), Reader view and saved passwords, in the real system image,
# against nexa-webdata. Screenshots: dist/browser-test.
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
for b in build/native/nexa-webfilter build/native/nexa-webdata; do
    [ -x "$repo/$b" ] || { echo "build it first: make native ($b)"; exit 1; }
done
mkdir -p "$repo/dist/browser-test"
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$repo/dist/browser-test:/out" nexa-sys:test sh /src/common/tests/browser_features.sh
