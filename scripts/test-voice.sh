#!/bin/sh
# Nexa Mind's voice and ears (tools-c/nexa-speak, tools-c/nexa-voice,
# tools/nexa-mindd): unit tests (C and Rust), then the real thing in a
# headless Sway: nexa-mindd serving Kokoro and whisper from real models,
# nexa-voice speaking with the helix on screen, and hearing Kokoro's own
# words back through whisper. Screenshots: OUT_DIR.
#
# The models (about 400 MB) are downloaded once to MODELS and checked
# against the SHA-256 in tools/nexa-mindd/src/catalog.rs.
# Usage: sh scripts/test-voice.sh [OUT_DIR]   (default dist/voice-test)
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
out=${1:-$repo/dist/voice-test}
models=${MODELS:-/var/tmp/nexa-models/mind}
mkdir -p "$out" "$repo/build/test" "$models" /var/tmp/nexa-cargo
out=$(cd "$out" && pwd)

echo "== models"
# name, file, url, sha256: read from the catalogue, so they can't drift apart
python3 - "$repo/tools/nexa-mindd/src/catalog.rs" > "$out/models.txt" << 'PY'
import re, sys
src = open(sys.argv[1]).read()
base = re.search(r'concat!\("(https://[^"]+/)", \$path\)', src).group(1)
for m in re.finditer(r'file: "([^"]+)",\s*url: (?:concat_kokoro!\("([^"]+)"\)|"([^"]+)"),\s*size: ([\d_]+),\s*sha256: "([0-9a-f]{64})"', src):
    file, kpath, url, size, sha = m.groups()
    if file.endswith(".gguf"):
        continue   # the chat model isn't needed here
    print(file, base + kpath if kpath else url, size.replace("_", ""), sha)
PY
while read -r file url size sha; do
    if [ "$(cat "$models/$file.verified" 2>/dev/null)" != "$sha" ] || [ "$(stat -c %s "$models/$file" 2>/dev/null)" != "$size" ]; then
        echo "fetching $file"
        curl -fsSL --proto =https -o "$models/$file.part" "$url"
        [ "$(sha256sum "$models/$file.part" | cut -d' ' -f1)" = "$sha" ] || { echo "checksum mismatch: $file"; exit 1; }
        mv "$models/$file.part" "$models/$file"
        echo "$sha" > "$models/$file.verified"
    fi
done < "$out/models.txt"

if ! docker image inspect "nexa-voicebuild:${version%.*}" >/dev/null 2>&1; then
    printf 'FROM nexa-rust-apps:%s\nRUN apk add --no-cache onnxruntime-dev espeak-ng whisper.cpp\n' "${version%.*}" \
        | docker build -q -t "nexa-voicebuild:${version%.*}" - >/dev/null
fi
if ! docker run --rm "nexa-voicetest:${version%.*}" sh -c 'command -v whisper-cli && command -v wtype' >/dev/null 2>&1; then
    printf 'FROM nexa-apptest:%s\nRUN apk add -q wtype gst-plugins-good espeak-ng onnxruntime whisper.cpp\n' "${version%.*}" \
        | docker build -q -t "nexa-voicetest:${version%.*}" - >/dev/null
fi

echo "== C: text to phonemes, the helix"
docker run --rm -v "$repo:/src" -e OWNER="$(id -u):$(id -g)" "nexa-voicebuild:${version%.*}" sh -c '
set -e
for t in nexa-speak:speak-test nexa-voice:voice-test; do
    d=${t%%:*}
    make -s -C /src/tools-c/$d test | grep -v "^PASS"
    make -s -C /src/tools-c/$d $d
    install -m 0755 /src/tools-c/$d/$d /src/build/test/$d
    rm -f /src/tools-c/$d/$d /src/tools-c/$d/${t#*:}
done
chown "$OWNER" /src/build/test/nexa-speak /src/build/test/nexa-voice
'
echo "== Rust: nexa-mindd"
docker run --rm -v "$repo:/src" -v /var/tmp/nexa-cargo:/root/.cargo/registry -e OWNER="$(id -u):$(id -g)" -w /src/tools "nexa-rust:${version%.*}" sh -c '
set -e
cargo test -q -p nexa-mindd 2>&1 | grep -E "^test result: ok. [1-9]|FAILED|panicked" || { cargo test -q -p nexa-mindd 2>&1 | tail -30; exit 1; }
target=$(rustc -vV | sed -n "s/^host: //p")
RUSTFLAGS="-C target-feature=+crt-static" cargo build --release -q -p nexa-mindd --target $target
install -m 0755 target/$target/release/nexa-mindd /src/build/test/
chown -R "$OWNER" /src/tools/target /src/tools/Cargo.lock /src/build/test
'

echo "== the voice"
docker run --rm --cap-add SYS_NICE -v "$repo:/src:ro" -v "$models:/tmp/mind-state/models:ro" -v "$out:/out" -e OUT=/out "nexa-voicetest:${version%.*}" sh -c '
export XDG_RUNTIME_DIR=/tmp/xdg WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 HOME=/root GSETTINGS_BACKEND=keyfile
mkdir -p $XDG_RUNTIME_DIR && chmod 700 $XDG_RUNTIME_DIR
# Programs go where the system keeps them: nexa-mindd'"'"'s Landlock rules
# let it run programs from /usr only.
for t in nexa-mindd nexa-speak nexa-voice; do install -m 0755 /src/build/test/$t /usr/local/bin/$t; done
# The service'"'"'s folders as on NexaOS; the models are mounted in place
# (a link out of the state folder would be refused by Landlock).
mkdir -p /tmp/mind-state /tmp/mind-run
printf "set \$mod Mod4\noutput * resolution 1280x800\n" > /tmp/sway.conf
sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for i in $(seq 50); do ls $XDG_RUNTIME_DIR/wayland-* >/dev/null 2>&1 && break; sleep 0.2; done
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.* | head -1) WAYLAND_DISPLAY=$(basename $(ls $XDG_RUNTIME_DIR/wayland-* | grep -v lock | head -1))
python3 /src/tools-c/nexa-voice/tests/headless_voice.py
'
