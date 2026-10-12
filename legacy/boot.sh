#!/bin/bash

set -euo pipefail

# NEXA OS Boot Script

BASE="${NEXA_BASE:-$HOME/NexaOS}"
MODE="${1:-serial}"
DISPLAY_BACKEND="${NEXA_DISPLAY:-gtk}"

CLASSIC_KERNEL="$BASE/kernel/linux-6.18.33/arch/x86/boot/bzImage"
CLASSIC_INITRD="$BASE/initramfs.cpio.gz"
CLASSIC_DISK="$BASE/nexa.img"

WAYLAND_KERNEL="$BASE/dist/bzImage"
WAYLAND_INITRD="$BASE/dist/initramfs.cpio.gz"
WAYLAND_DISK="$BASE/dist/nexa.img"

usage() {
    cat <<EOF
Usage: ./boot.sh [serial|gui|wayland]

serial   Boot classic Nexa in the terminal with -nographic. Default.
gui      Boot classic Nexa with virtio-gpu; run 'gui' at nexa>.
wayland  Boot the modern dist image with DRM/input devices for Sway/Wayland.

Environment:
  NEXA_BASE     Override OS tree path. Default: $HOME/NexaOS
  NEXA_DISPLAY  QEMU display backend for gui/wayland. Default: gtk
EOF
}

require_file() {
    local path="$1"
    local label="$2"
    local hint="${3:-}"

    if [ ! -f "$path" ]; then
        echo "ERROR: $label not found: $path"
        if [ -n "$hint" ]; then
            echo "$hint"
        fi
        exit 1
    fi
}

display_arg() {
    case "$DISPLAY_BACKEND" in
        none|curses|*,*) printf "%s" "$DISPLAY_BACKEND" ;;
        *) printf "%s,show-cursor=on" "$DISPLAY_BACKEND" ;;
    esac
}

warn_if_wayland_image_is_stale() {
    if ! command -v debugfs >/dev/null 2>&1; then
        return 0
    fi

    if ! debugfs -R "stat /usr/bin/sway" "$DISK" >/dev/null 2>&1; then
        echo "WARN: $DISK does not appear to contain /usr/bin/sway yet."
        echo "      Rebuild it with: sudo make disk"
        echo
    fi
}

case "$MODE" in
    serial|text|nographic)
        MODE="serial"
        ;;

    gui|graphical|desktop|framebuffer|fb)
        MODE="gui"
        ;;

    wayland|sway|modern)
        MODE="wayland"
        ;;

    help|-h|--help)
        usage
        exit 0
        ;;

    *)
        echo "Unknown boot mode: $MODE"
        usage
        exit 1
        ;;
esac

if [ "$MODE" = "wayland" ]; then
    KERNEL="$WAYLAND_KERNEL"
    INITRD="$WAYLAND_INITRD"
    DISK="$WAYLAND_DISK"
    APPEND="root=/dev/vda console=tty0 rootwait"
    DRIVE_ARGS=( -drive "if=none,id=nexadisk,file=$DISK,format=raw" -device virtio-blk-pci,drive=nexadisk )
    NET_ARGS=( -netdev user,id=net0,hostfwd=tcp::2222-:22 -device virtio-net-pci,netdev=net0 )
else
    KERNEL="$CLASSIC_KERNEL"
    INITRD="$CLASSIC_INITRD"
    DISK="$CLASSIC_DISK"
    if [ "$MODE" = "gui" ]; then
        APPEND="console=tty0 console=ttyS0 rdinit=/init"
    else
        APPEND="console=ttyS0 rdinit=/init"
    fi
    DRIVE_ARGS=( -drive "file=$DISK,format=raw" )
    NET_ARGS=( -netdev user,id=net0,hostfwd=tcp::2222-:22 -device e1000,netdev=net0 )
fi

echo "================================="
echo "  NEXA OS - BOOT SCRIPT"
echo "================================="
echo

if [ "$MODE" = "wayland" ]; then
    BUILD_HINT="Build the modern image first with: make disk"
else
    BUILD_HINT="Build or restore the classic image first."
fi

require_file "$DISK" "disk image" "$BUILD_HINT"
require_file "$KERNEL" "kernel" "$BUILD_HINT"
require_file "$INITRD" "initramfs" "$BUILD_HINT"

if [ "$MODE" = "wayland" ]; then
    warn_if_wayland_image_is_stale
fi

echo "OK: All components found"
echo
echo "Starting QEMU with:"
echo "  Mode:      $MODE"
echo "  Kernel:    $KERNEL"
echo "  Initramfs: $INITRD"
echo "  Disk:      $DISK"
echo "  Append:    $APPEND"

if [ "$MODE" = "gui" ]; then
    echo "  Display:   $DISPLAY_BACKEND"
    echo
    echo "A QEMU display window will open."
    echo "Run 'gui' inside Nexa to draw the framebuffer desktop."
elif [ "$MODE" = "wayland" ]; then
    echo "  Display:   $DISPLAY_BACKEND"
    echo
    echo "A QEMU display window will open."
    echo "Inside Nexa, use 'nexa-wayland-check' or 'nexa-session' to inspect/start the UI."
else
    echo
    echo "Press Ctrl+A then X to exit QEMU"
fi

echo
echo "================================="
sleep 2

if [ "$MODE" = "gui" ]; then
    qemu-system-x86_64 \
      -kernel "$KERNEL" \
      -initrd "$INITRD" \
      -append "$APPEND" \
      "${DRIVE_ARGS[@]}" \
      "${NET_ARGS[@]}" \
      -device virtio-gpu-pci,xres=1024,yres=768 \
      -audiodev pa,id=snd0 \
      -device AC97,audiodev=snd0 \
      -display "$(display_arg)" \
      -serial mon:stdio
elif [ "$MODE" = "wayland" ]; then
    qemu-system-x86_64 \
      -kernel "$KERNEL" \
      -initrd "$INITRD" \
      -append "$APPEND" \
      "${DRIVE_ARGS[@]}" \
      "${NET_ARGS[@]}" \
      -device virtio-gpu-pci,xres=1280,yres=800 \
      -device virtio-mouse-pci \
      -device virtio-keyboard-pci \
      -display "$(display_arg)" \
      -serial mon:stdio
else
    qemu-system-x86_64 \
      -kernel "$KERNEL" \
      -initrd "$INITRD" \
      -append "$APPEND" \
      "${DRIVE_ARGS[@]}" \
      "${NET_ARGS[@]}" \
      -audiodev pa,id=snd0 \
      -device AC97,audiodev=snd0 \
      -nographic
fi
