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
# The Rust builder plus the headers apps commonly need beyond GTK, saved
# once (nexa-rust-apps) instead of downloaded on every build.
apps_image=nexa-rust-apps:${version%.*}
if ! docker image inspect "$apps_image" >/dev/null 2>&1; then
    printf 'FROM nexa-rust:%s\nRUN apk add --no-cache gstreamer-dev gst-plugins-base-dev\n' "${version%.*}" \
        | docker build -q -t "$apps_image" - >/dev/null
fi
docker run --rm -v "$repo:/src" -v "$keys:/keys:ro" -e OWNER="$(id -u):$(id -g)" "$apps_image" sh -c '
set -e
key=""; [ -f /keys/nexaos-release.key ] && key="--key /keys/nexaos-release.key"
for app in /src/apps/*/nexa.toml; do
    dir=${app%/nexa.toml}
    (cd "$dir" && rm -rf build dist && /src/build/native/nexa-pack build $key | tail -1)
    chown -R "$OWNER" "$dir/build" "$dir/dist"
done
# The Nexa Store'"'"'s built-in catalogue: every app above, signed like them
# (nexa-pack repo build); shipped in the image as /usr/share/nexa-store/repo.
rm -rf /src/build/store-repo
mkdir -p /src/build/store-repo/packages
cp /src/apps/*/dist/*.nexa /src/build/store-repo/packages/
/src/build/native/nexa-pack repo build /src/build/store-repo $key --keys /src/assets/keys
chown -R "$OWNER" /src/build/store-repo
'
