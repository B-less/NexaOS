#!/bin/bash
set -e
BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"

umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT

# Add TERMINFO to /etc/profile if not present
if ! grep -q "TERMINFO=" $MNT/etc/profile; then
    sed -i '1s/^/export TERMINFO=\/usr\/share\/terminfo\n/' $MNT/etc/profile
fi

# Ensure xterm-256color actually exists
mkdir -p $MNT/usr/share/terminfo/x
cp /usr/share/terminfo/x/xterm-256color $MNT/usr/share/terminfo/x/ 2>/dev/null || true

umount $MNT
echo "TERMINFO environment variable injected."
