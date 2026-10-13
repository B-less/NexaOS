#!/bin/sh
# Build NexaOS's native (Rust) tools in an Alpine container as static
# binaries, so they run on NexaOS and on the build host, and a root helper
# depending on one never depends on a shared library.
#   sh scripts/build-native.sh      -> build/native/<tool>
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$repo/build/native" /var/tmp/nexa-cargo
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
docker run --rm -v "$repo:/src" -v /var/tmp/nexa-cargo:/root/.cargo/registry \
    -e RUSTFLAGS="-C target-feature=+crt-static" "alpine:${version%.*}" sh -c '
set -e
apk add -q cargo rust >/dev/null 2>&1
for tool in /src/tools/*/; do
    [ -f "$tool/Cargo.toml" ] || continue
    name=$(basename "$tool")
    # An explicit --target keeps the static flag off build-time macro crates.
    target=$(rustc -vV | sed -n "s/^host: //p")
    (cd "$tool" && cargo build --release -q --target "$target")
    install -m 0755 "$tool/target/$target/release/$name" /src/build/native/$name
    echo "built $name: $(stat -c %s /src/build/native/$name) bytes"
done
chown -R '"$(id -u):$(id -g)"' /src/build/native /src/tools/*/target 2>/dev/null || true
'
