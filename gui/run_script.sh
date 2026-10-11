#!/bin/sh

APP_DIR="/disk/apps/desktop-gui"

if [ ! -e /dev/fb0 ]; then
    mkdir -p /dev
    mknod /dev/fb0 c 29 0 2>/dev/null
fi

if [ ! -e /dev/fb0 ]; then
    echo "No framebuffer device found."
    echo "Boot with ./boot-gui.sh first."
    exit 1
fi

if [ ! -x "$APP_DIR/fbdesktop" ]; then
    echo "Framebuffer desktop binary not found."
    exit 1
fi

"$APP_DIR/fbdesktop"
