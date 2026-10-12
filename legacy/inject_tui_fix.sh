#!/bin/bash
set -e
BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"

umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT

# Copy ncurses include directory itself so ncurses/ncurses.h works
mkdir -p $MNT/usr/lib/tcc/include/
cp -r /tmp/ncurses-build/include/ncurses $MNT/usr/lib/tcc/include/

# Copy the TUI source code
cp /mnt/c/Users/BBF/nexa_tui.c $MNT/home/nexa/nexa_tui.c
chown 1000:1000 $MNT/home/nexa/nexa_tui.c

umount $MNT
echo "TUI and headers fixed."
