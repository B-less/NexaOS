#!/bin/sh
# The build container image (nexa-build:<alpine>): Alpine plus every tool the
# build steps need (bash for provisioning, e2fsprogs for the disk image,
# xorriso/syslinux/grub/mtools/dosfstools for the ISO), downloaded once.
# Without it each `make refresh` downloaded them again into throwaway
# containers (~34 MB). To pick up newer tools: docker rmi nexa-build:<alpine>
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
tag=${version%.*}
image=nexa-build:$tag
docker image inspect "$image" >/dev/null 2>&1 && exit 0
echo "[*] building the $image build image (one-time download, ~25 MB)"
for try in 1 2 3 4 5; do
    printf 'FROM alpine:%s\nRUN apk add --no-cache bash e2fsprogs xorriso syslinux grub grub-efi mtools dosfstools\n' "$tag" \
        | docker build -q -t "$image" - >/dev/null && exit 0
    echo "[*] download failed (network?), retrying in 5 s ($try/5)"
    sleep 5
done
echo "ERROR: couldn't build $image (no internet?)" >&2
exit 1
