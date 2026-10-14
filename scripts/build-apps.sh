#!/bin/sh
# Build every NexaOS app under apps/ into a .nexa package (apps/<id>/dist/).
# Signed with the NexaOS release key when this machine has it
# (~/.config/nexaos-keys/nexaos-release.key, never in the repository),
# unsigned otherwise. Runs in the nexa-rust builder image.
# Usage: sh scripts/build-apps.sh
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
keys="$HOME/.config/nexaos-keys"
[ -x "$repo/build/native/nexa-pack" ] || sh "$repo/scripts/build-native.sh" >/dev/null
docker run --rm -v "$repo:/src" -v "$keys:/keys:ro" -e OWNER="$(id -u):$(id -g)" "nexa-rust:${version%.*}" sh -c '
set -e
# Headers apps commonly need beyond GTK (the image has gtk+3.0-dev).
apk add -q gstreamer-dev gst-plugins-base-dev >/dev/null
key=""; [ -f /keys/nexaos-release.key ] && key="--key /keys/nexaos-release.key"
for app in /src/apps/*/nexa.toml; do
    dir=${app%/nexa.toml}
    (cd "$dir" && rm -rf build dist && /src/build/native/nexa-pack build $key | tail -1)
    chown -R "$OWNER" "$dir/build" "$dir/dist"
done
'
