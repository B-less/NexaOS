#!/bin/sh
# Build NexaOS's native C tools (tools-c/*) -> build/native/, in the
# nexa-rust-apps image (Alpine, same libraries as the system image: GTK,
# gdk-pixbuf, GStreamer). Staged into the image with the Rust tools.
#   sh scripts/build-c-tools.sh
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-rust-apps:${version%.*}
# Rebuilt when a needed package is missing (linux-pam-dev: nexa-locker;
# wlr-protocols: nexa-barhide's layer-shell surface; poppler-dev: nexa-pdfdoc;
# gtk-layer-shell-dev: nexa-orbit).
if ! docker run --rm "$image" sh -c 'apk info -e linux-pam-dev && apk info -e wlr-protocols && apk info -e poppler-dev && apk info -e gtk-layer-shell-dev' >/dev/null 2>&1; then
    docker image inspect "nexa-rust:${version%.*}" >/dev/null 2>&1 || sh "$repo/scripts/build-native.sh" >/dev/null
    printf 'FROM nexa-rust:%s\nRUN apk add --no-cache gstreamer-dev gst-plugins-base-dev linux-pam-dev wlr-protocols poppler-dev gtk-layer-shell-dev\n' "${version%.*}" \
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
# Settings draws its style previews with the renderer'"'"'s own code.
make -s -C /src/tools-c/nexa-living nexa-living-preview
install -m 0755 /src/tools-c/nexa-living/nexa-living-preview /src/build/native/nexa-living-preview
rm -f /src/tools-c/nexa-living/nexa-living-preview
echo "built nexa-living-preview: $(stat -c %s /src/build/native/nexa-living-preview) bytes"
# Test helpers: build/test, never staged into the image.
mkdir -p /src/build/test /tmp/vp
wayland-scanner client-header /usr/share/wlr-protocols/unstable/wlr-virtual-pointer-unstable-v1.xml /tmp/vp/virtual-pointer.h
wayland-scanner private-code /usr/share/wlr-protocols/unstable/wlr-virtual-pointer-unstable-v1.xml /tmp/vp/virtual-pointer.c
cc -O2 -Wall -Wextra -Wno-unused-parameter -I/tmp/vp -o /src/build/test/nexa-vpointer /src/common/tests/c/vpointer.c /tmp/vp/virtual-pointer.c $(pkg-config --cflags --libs wayland-client)
echo "built nexa-vpointer (tests only): $(stat -c %s /src/build/test/nexa-vpointer) bytes"
chown -R "$OWNER" /src/build/native /src/build/test
'
