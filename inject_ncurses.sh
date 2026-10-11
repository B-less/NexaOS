#!/bin/bash
set -e
BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"

umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT

cp -r /tmp/ncurses-build/include/ncurses/* $MNT/usr/lib/tcc/include/
cp -r /tmp/ncurses-build/lib/*.a $MNT/usr/lib/x86_64-linux-gnu/

mkdir -p $MNT/usr/share/terminfo/l
mkdir -p $MNT/usr/share/terminfo/v
mkdir -p $MNT/usr/share/terminfo/x
cp /usr/share/terminfo/l/linux $MNT/usr/share/terminfo/l/ || true
cp /usr/share/terminfo/v/vt100 $MNT/usr/share/terminfo/v/ || true
cp /usr/share/terminfo/x/xterm* $MNT/usr/share/terminfo/x/ || true

umount $MNT
echo "Injection complete"
