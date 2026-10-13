#!/bin/sh
# Stage the native (Rust) tools from build/native into the rootfs, plus the
# package trust store: /etc/nexa/keys holds the public keys of publishers
# whose .nexa packages install without a warning (assets/keys/*.pub; the
# matching secret keys never leave the publisher's machine).
set -eu
rootfs=${1:-rootfs}
base=${NEXAOS_BASE:-/home/nexa/NexaOS}
for bin in "$base"/build/native/*; do
    [ -f "$bin" ] || continue
    install -D -m 0755 "$bin" "$rootfs/usr/local/bin/$(basename "$bin")"
    echo "staged $(basename "$bin")"
done
mkdir -p "$rootfs/etc/nexa/keys"
for key in "$base"/assets/keys/*.pub; do
    [ -f "$key" ] && install -m 0644 "$key" "$rootfs/etc/nexa/keys/$(basename "$key")"
done
chmod 0755 "$rootfs/etc/nexa/keys"
