#!/bin/sh
# Build NexaOS's native C tools (tools-c/*) -> build/native/, in the
# nexa-rust-apps image (Alpine, same libraries as the system image: GTK,
# gdk-pixbuf, GStreamer). Staged into the image with the Rust tools.
#   sh scripts/build-c-tools.sh
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-rust-apps:${version%.*}
# Rebuilt when a needed package is missing (linux-pam-dev: nexa-locker).
if ! docker run --rm "$image" apk info -e linux-pam-dev >/dev/null 2>&1; then
    docker image inspect "nexa-rust:${version%.*}" >/dev/null 2>&1 || sh "$repo/scripts/build-native.sh" >/dev/null
    printf 'FROM nexa-rust:%s\nRUN apk add --no-cache gstreamer-dev gst-plugins-base-dev linux-pam-dev\n' "${version%.*}" \
        | docker build -q -t "$image" - >/dev/null
fi
mkdir -p "$repo/build/native"
docker run --rm -v "$repo:/src" -e OWNER="$(id -u):$(id -g)" "$image" sh -c '
set -e
for dir in /src/tools-c/*/; do
    name=$(basename "$dir")
    make -s -C "$dir" "$name"
    install -m 0755 "$dir/$name" "/src/build/native/$name"
    rm -f "$dir/$name"
    echo "built $name: $(stat -c %s /src/build/native/$name) bytes"
done
chown -R "$OWNER" /src/build/native
'
