#!/bin/sh
# NexaLang's whole test suite (nexalang/: cargo test), in an Alpine
# container that has everything the tests look for, so none is skipped:
# GTK 3 and GtkSourceView 4 (window programs), libcurl (`use network`),
# Xvfb + xdotool (driving windows), valgrind (memory), ImageMagick
# (screenshots), openssl (a test HTTPS server's certificate), the icon
# themes NexaOS has.
# On a machine without them, `cargo test` still runs but skips those tests.
# Usage: sh scripts/test-nexalang.sh [cargo test arguments]
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-langtest:${version%.*}
need='gtk+3.0-dev gtksourceview4-dev curl-dev pkgconf xvfb xdotool valgrind imagemagick openssl python3 font-inter font-jetbrains-mono font-dejavu hicolor-icon-theme adwaita-icon-theme'
if ! docker run --rm "$image" sh -c "apk info -e $need" >/dev/null 2>&1; then
    echo "making $image (once)"
    printf 'FROM nexa-rust:%s\nRUN apk add --no-cache %s\n' "${version%.*}" "$need" \
        | docker build -q -t "$image" - >/dev/null
fi
mkdir -p "$repo/build/nexalang-target"
docker run --rm -v "$repo:/src" -w /src/nexalang \
    -e CARGO_TARGET_DIR=/src/build/nexalang-target -e CARGO_HOME=/tmp/cargo -e HOME=/tmp \
    "$image" cargo test --release "$@"
