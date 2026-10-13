#!/bin/sh
# End-to-end test of nexa-pack in a clean Alpine container: build the tool,
# make a publisher key, create + build + sign both templates (C/GTK and
# Rust), verify with and without the key trusted, catch a tampered package,
# and run the Rust app from an extracted package.
# Usage: sh scripts/test-nexa-pack.sh
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p /var/tmp/nexa-cargo
docker run --rm -v "$repo/tools/nexa-pack:/src:ro" -v /var/tmp/nexa-cargo:/root/.cargo/registry alpine:3.24 sh -c '
set -e
apk add -q cargo rust gcc musl-dev make pkgconf gtk+3.0-dev zip unzip >/dev/null 2>&1
cp -r /src /build && cd /build && cargo build --release -q 2>&1 | tail -3
P=/build/target/release/nexa-pack
fail() { echo "FAIL: $*"; exit 1; }
cd /tmp
$P keygen publisher >/dev/null
ID=$(head -1 publisher.pub | cut -d" " -f3)
mkdir -p keys && cp publisher.pub keys/$ID.pub

$P new hello-c --lang c >/dev/null
(cd hello-c && $P build --key /tmp/publisher.key >/dev/null) || fail "C template did not build"
C=hello-c/dist/hello-c-0.1.0.nexa
$P verify $C --keys keys --json | grep -q "\"trust\":\"trusted\"" || fail "signed C package not trusted"
echo "ok  C template builds, signs and verifies as trusted"

$P verify $C --keys /nonexistent --json | grep -q "\"trust\":\"unknown-publisher\"" || fail "unknown key not reported"
echo "ok  same package with the key not trusted: unknown-publisher"

(cd hello-c && $P build --no-build -o /tmp/again.nexa --key /tmp/publisher.key >/dev/null)
cmp -s $C /tmp/again.nexa || fail "packages are not reproducible"
echo "ok  rebuilding gives a byte-identical package"

mkdir t && cd t && unzip -q ../$C && echo "#!/bin/sh" > bin/hello-c && zip -q -r ../tampered.nexa . && cd ..
if $P verify tampered.nexa --keys keys >/dev/null 2>&1; then fail "tampered package accepted"; fi
echo "ok  tampered package rejected: $($P verify tampered.nexa --keys keys 2>&1 | tail -1)"

$P new sysinfo --lang rust >/dev/null
(cd sysinfo && $P build --key /tmp/publisher.key >/dev/null 2>&1) || fail "Rust template did not build"
$P extract sysinfo/dist/sysinfo-0.1.0.nexa /tmp/out --keys keys --json | grep -q "\"ok\":true" || fail "extract failed"
/tmp/out/bin/sysinfo | grep -q "Hello from NexaOS" || fail "extracted Rust app did not run"
echo "ok  Rust template builds, extracts and runs: $(/tmp/out/bin/sysinfo | sed -n 3p)"
[ "$(stat -c %a /tmp/out/bin/sysinfo)" = 755 ] || fail "entry point not executable"
if $P extract sysinfo/dist/sysinfo-0.1.0.nexa /tmp/out --keys keys >/dev/null 2>&1; then fail "extract reused a directory"; fi
echo "ok  extract refuses an existing directory"
echo PASS
'
