#!/bin/sh
# Rewind (Magic Layer) end to end: the real nexa-microbrain watching real
# settings files changed the way Settings changes them; going back, the
# desktop following, undoing the going back, what is never tracked.
# Usage: sh scripts/test-rewind.sh   (needs build/native/nexa-microbrain: make native)
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
[ -x "$repo/build/native/nexa-microbrain" ] || { echo "build it first: make native"; exit 1; }
docker run --rm -v "$repo:/src:ro" "nexa-apptest:${version%.*}" sh /src/common/tests/rewind_e2e.sh
