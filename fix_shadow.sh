#!/bin/bash
set -e

HASH='$1$NFatqZ.h$o1lYUUVKL9GhokkX2x8ej/'

umount /home/nexa/NexaOS/mnt 2>/dev/null || true
mount -o loop /home/nexa/NexaOS/nexa.img /home/nexa/NexaOS/mnt

# Write correct shadow with real MD5 hash for "password"
printf 'root:%s:19000:0:99999:7:::\n' "$HASH" > /home/nexa/NexaOS/mnt/etc/shadow
printf 'nexa:%s:19000:0:99999:7:::\n' "$HASH" >> /home/nexa/NexaOS/mnt/etc/shadow
chmod 600 /home/nexa/NexaOS/mnt/etc/shadow

echo "=== /etc/shadow ==="
cat /home/nexa/NexaOS/mnt/etc/shadow

umount /home/nexa/NexaOS/mnt
echo "Done."
