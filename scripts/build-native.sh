#!/bin/sh
# Build NexaOS's native (Rust) tools as static binaries -> build/native/.
#   sh scripts/build-native.sh           release build of every tool
#   sh scripts/build-native.sh test      cargo test for the whole workspace
#
# Fast on repeat: a builder image with Rust already installed (nexa-rust),
# one Cargo workspace (shared dependencies compile once) and a persistent
# target folder (tools/target), so only what changed recompiles.
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
mode=${1:-build}
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-rust:${version%.*}
if ! docker image inspect "$image" >/dev/null 2>&1; then
    printf 'FROM alpine:%s\nRUN apk add -q cargo rust gcc musl-dev make pkgconf gtk+3.0-dev zip unzip\n' "${version%.*}" \
        | docker build -t "$image" -
fi
mkdir -p "$repo/build/native" /var/tmp/nexa-cargo
docker run --rm -v "$repo:/src" -v /var/tmp/nexa-cargo:/root/.cargo/registry \
    -e MODE="$mode" -e OWNER="$(id -u):$(id -g)" -w /src/tools "$image" sh -c '
set -e
target=$(rustc -vV | sed -n "s/^host: //p")
if [ "$MODE" = test ]; then
    rc=0; cargo test --workspace -q >/tmp/test.log 2>&1 || rc=$?
    grep -E "^test result|FAILED|panicked|^error" /tmp/test.log || true
    [ $rc = 0 ] || { tail -40 /tmp/test.log; exit $rc; }
else
    # An explicit --target keeps the static flag off build-time macro crates.
    RUSTFLAGS="-C target-feature=+crt-static" cargo build --release --workspace -q --target "$target"
    # Each crate'"'"'s main binary, plus any extra ones (src/bin/*.rs, such as
    # nexa-signalctl beside nexa-signald).
    for main in */src/main.rs */src/bin/*.rs; do
        [ -e "$main" ] || continue
        case $main in */src/bin/*) name=$(basename "$main" .rs) ;; *) name=${main%%/*} ;; esac
        install -m 0755 "target/$target/release/$name" /src/build/native/$name
        echo "built $name: $(stat -c %s /src/build/native/$name) bytes"
    done
fi
chown -R "$OWNER" /src/tools/target /src/tools/Cargo.lock /src/build/native 2>/dev/null || true
'
