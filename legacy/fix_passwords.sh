#!/bin/bash
set -e

BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"

umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT

echo "/bin/sh" > $MNT/etc/shells
echo "/bin/ash" >> $MNT/etc/shells

NEXA_HASH=$(openssl passwd -1 -salt nexa nexa)
ROOT_HASH=$(openssl passwd -1 -salt nexa root)

sed -i "s|^nexa:.*|nexa:$NEXA_HASH:19000:0:99999:7:::|" $MNT/etc/shadow
sed -i "s|^root:.*|root:$ROOT_HASH:19000:0:99999:7:::|" $MNT/etc/shadow

umount $MNT
echo "Passwords fixed."
