#!/bin/sh
# SETUP for test-app-headless.sh: builds vpointer, then drags the app's
# window by its title bar and prints its position before and after.
# DRAG_ONLY=<app_id> closes every other window first; DRAG_X/DRAG_Y is where
# the mouse grabs (default 400,118: the title bar of a window at 100,100).
cd /tmp
P=/usr/share/wlr-protocols/unstable/wlr-virtual-pointer-unstable-v1.xml
wayland-scanner client-header $P wlr-virtual-pointer-unstable-v1-client-protocol.h
wayland-scanner private-code $P vp.c
gcc -O2 -I. -o /tmp/vpointer /src/scripts/vpointer.c vp.c -lwayland-client || exit 1
(
sleep 5
pos() { swaymsg -t get_tree | python3 -c 'import json,sys
def walk(n):
    if n.get("app_id"): print("POS", n["app_id"], n["rect"]["x"], n["rect"]["y"])
    for c in n.get("nodes", []) + n.get("floating_nodes", []): walk(c)
walk(json.load(sys.stdin))'; }
# DRAG_ONLY=<app_id>: close every other window first.
[ -n "${DRAG_ONLY:-}" ] && swaymsg "[app_id=\"^(?!${DRAG_ONLY}\$).*\"] kill" >/dev/null
swaymsg '[app_id=".*"] floating enable, resize set 800 500, move position 100 100' >/dev/null
sleep 1; echo BEFORE; pos
/tmp/vpointer 1280 800 abs ${DRAG_X:-400} ${DRAG_Y:-118} sleep 300 press sleep 100 \
    move 20 15 move 20 15 move 20 15 move 20 15 move 20 15 move 20 15 move 20 15 move 20 15 move 20 15 move 20 15 \
    sleep 200 release
sleep 1; echo AFTER; pos
) &
