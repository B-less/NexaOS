#!/bin/sh
# Why (Magic Layer) end to end: the real nexa-microbrain service on a real
# /proc, apps started the way the desktop starts them, real crashes, kills,
# a paused program, a busy loop, real networking and none at all; then the
# Why tab in Nexa Pulse in a headless Sway (screenshot in dist/why-test).
# Usage: sh scripts/test-why.sh   (needs build/native/nexa-microbrain: make native)
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-apptest:${version%.*}
[ -x "$repo/build/native/nexa-microbrain" ] || { echo "build it first: make native"; exit 1; }
out=${1:-$repo/dist/why-test}
mkdir -p "$out"
rc=0
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$(cd "$out" && pwd):/out" "$image" sh /src/common/tests/why_e2e.sh || rc=1
# No network at all: the honest answer is that there is no adapter.
docker run --rm --network none -v "$repo:/src:ro" "$image" sh /src/common/tests/why_e2e.sh offline || rc=1
exit $rc
