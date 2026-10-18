#!/bin/bash
# Generate configuration files for NexaOS
set -e

ROOTFS_DIR="${1:-.}"
MNT_DIR="${2:-.}"

# Check if running for rootfs or mounted disk
IS_ROOTFS=true
[[ -d "$MNT_DIR/etc" && "$MNT_DIR" != "." ]] && IS_ROOTFS=false

echo "[*] Generating init script..."
cat > "$ROOTFS_DIR/init" << 'INIT_EOF'
#!/bin/sh
PATH=/sbin:/bin:/usr/sbin:/usr/bin

mkdir -p /proc /sys /dev /run /tmp /mnt/root

mount -t proc proc /proc 2>/dev/null || echo "[INIT] WARN: could not mount proc"
mount -t sysfs sysfs /sys 2>/dev/null || echo "[INIT] WARN: could not mount sysfs"
mount -t devtmpfs devtmpfs /dev 2>/dev/null || echo "[INIT] WARN: could not mount devtmpfs"
mkdir -p /dev/pts
mount -t devpts devpts /dev/pts 2>/dev/null || true

if [ -e /proc/sys/kernel/hotplug ]; then
    ( echo /sbin/mdev > /proc/sys/kernel/hotplug ) 2>/dev/null || true
fi

if [ -x /sbin/mdev ]; then
    /sbin/mdev -s 2>/dev/null || true
fi

ensure_block_node() {
    DEV="$1"
    [ -b "/dev/$DEV" ] && return 0

    # /sys/block/$DEV only has a "dev" file for a *whole disk* (e.g.
    # /sys/block/sda/dev). A partition's is under its parent instead
    # (/sys/block/sda/sda3/dev, not /sys/block/sda3/dev) -- /sys/class/block
    # is the flat namespace that has every disk AND every partition as a
    # direct entry, so it resolves both uniformly. Found by hand: an
    # installed system failed to boot on a non-virtio disk because this
    # fallback silently did nothing for the root partition specifically.
    SYSDEV_FILE="/sys/class/block/$DEV/dev"
    if [ -f "$SYSDEV_FILE" ]; then
        MAJOR=$(cut -d: -f1 "$SYSDEV_FILE")
        MINOR=$(cut -d: -f2 "$SYSDEV_FILE")
        echo "[INIT] Creating /dev/$DEV ($MAJOR:$MINOR)"
        mknod "/dev/$DEV" b "$MAJOR" "$MINOR" 2>/dev/null || true
    fi
}

try_mount_root() {
    DEV="$1"

    ensure_block_node "$DEV"
    [ -b "/dev/$DEV" ] || return 1

    echo "[INIT] Attempting to mount /dev/$DEV..."
    if mount -t ext4 "/dev/$DEV" /mnt/root 2>/dev/null; then
        echo "[INIT] Successfully mounted /dev/$DEV"

        if [ -x /mnt/root/sbin/init ]; then
            echo "[INIT] Found /sbin/init, executing switch_root..."
            exec switch_root /mnt/root /sbin/init
        fi

        echo "[INIT] ERROR: /sbin/init not found in /mnt/root"
        umount /mnt/root 2>/dev/null || true
    fi

    return 1
}

# Wait for block device to initialize (IDE, VirtIO, SATA, or NVMe)
echo "[INIT] Waiting for block device..."
TIMEOUT=20
COUNTER=0
while [ $COUNTER -lt $TIMEOUT ]; do
    if grep -qE " (sd|vd|hd|xvd)[a-z]$| nvme[0-9]+n[0-9]+$" /proc/partitions 2>/dev/null; then
        echo "[INIT] Found block device in /proc/partitions"
        break
    fi
    if ls /sys/block/vd* /sys/block/sd* /sys/block/hd* /sys/block/xvd* /sys/block/nvme*n* >/dev/null 2>&1; then
        echo "[INIT] Found block device in /sys/block"
        break
    fi
    sleep 1
    COUNTER=$((COUNTER + 1))
done

echo "[INIT] Block devices in /proc/partitions:"
cat /proc/partitions 2>/dev/null | grep -v "major" || true

echo "[INIT] Block devices in /sys/block:"
ls /sys/block 2>/dev/null || true

ROOT_FROM_CMDLINE=$(sed -n 's/.*root=\([^ ]*\).*/\1/p' /proc/cmdline 2>/dev/null | sed 's#^/dev/##')
if [ -n "$ROOT_FROM_CMDLINE" ]; then
    echo "[INIT] Trying root= from cmdline: /dev/$ROOT_FROM_CMDLINE"
    try_mount_root "$ROOT_FROM_CMDLINE"
fi

for DEV in vda sda hda xvda nvme0n1; do
    try_mount_root "$DEV"
done

for SYSDEV in /sys/block/vd* /sys/block/sd* /sys/block/hd* /sys/block/xvd* /sys/block/nvme*n*; do
    [ -e "$SYSDEV" ] || continue
    DEV=$(basename "$SYSDEV")
    case "$DEV" in
        loop*|ram*|sr*|fd*) continue ;;
    esac
    try_mount_root "$DEV"

    # The disk itself can be visible here before the kernel has finished
    # registering its partitions (seen in practice: virtio disks enumerate
    # fast enough that this never showed up, but a plain IDE/AHCI-attached
    # disk under emulation left this loop with zero partition entries for a
    # moment). Give it a few short retries rather than scanning once and
    # moving on.
    PART_TRIES=0
    while [ "$PART_TRIES" -lt 10 ]; do
        FOUND_PART=
        for PART in "$SYSDEV"/"$DEV"*; do
            [ -e "$PART/dev" ] || continue
            FOUND_PART=1
            PARTDEV=$(basename "$PART")
            try_mount_root "$PARTDEV"
        done
        [ -n "$FOUND_PART" ] && break
        PART_TRIES=$((PART_TRIES + 1))
        sleep 0.5
    done
done

for DEVPATH in /dev/vd* /dev/sd* /dev/hd* /dev/xvd* /dev/nvme*n*; do
    [ -b "$DEVPATH" ] || continue
    DEV=$(basename "$DEVPATH")
    case "$DEV" in
        loop*|ram*|sr*|fd*) continue ;;
    esac
    try_mount_root "$DEV"
done

if ls /dev/[0-9]*p0 >/dev/null 2>&1; then
    echo "[INIT] Found unusual block aliases:"
    ls -l /dev/[0-9]*p0 2>/dev/null || true
    for DEVPATH in /dev/[0-9]*p0; do
        [ -b "$DEVPATH" ] || continue
        try_mount_root "$(basename "$DEVPATH")"
    done
fi

# Live-ISO fallback: no direct block-device root was found, which is normal
# when booting from an optical drive (VirtualBox et al present the ISO as a
# CD-ROM, not a disk with an ext4 filesystem on it). The ISO carries the same
# nexa.img this script mounts directly on real disk boots, just embedded as a
# plain file at its root -- loop-mount that file read-only and layer a tmpfs
# on top via overlayfs so the session is normally writable but discards
# changes on reboot, like any other live CD.
echo "[INIT] No direct root device found; trying live-ISO (CD) boot..."
mkdir -p /mnt/cdrom
for CDDEV in sr0 sr1 scd0 hdc hdd; do
    ensure_block_node "$CDDEV"
    [ -b "/dev/$CDDEV" ] || continue
    echo "[INIT] Trying CD-ROM /dev/$CDDEV..."
    if mount -t iso9660 -o ro "/dev/$CDDEV" /mnt/cdrom 2>/dev/null; then
        echo "[INIT] Mounted /dev/$CDDEV"
        break
    fi
done

if [ -f /mnt/cdrom/nexa.img ]; then
    echo "[INIT] Found nexa.img on CD, setting up loop + overlay..."
    mkdir -p /mnt/lower /mnt/live
    # noload: nexa.img's ext4 journal may be "dirty" (not cleanly unmounted --
    # a previous disk boot that was powered off rather than shut down leaves
    # it this way) and a read-only mount can't replay a journal to clean it,
    # so without this a dirty image would refuse to mount at all here. Safe
    # to skip replay: this filesystem is only ever the read-only lower layer
    # of the live overlay below, never written to directly.
    if losetup /dev/loop0 /mnt/cdrom/nexa.img 2>/dev/null \
        && mount -t ext4 -o ro,noload /dev/loop0 /mnt/lower 2>/dev/null; then
        mount -t tmpfs -o size=1G tmpfs /mnt/live
        mkdir -p /mnt/live/upper /mnt/live/work
        if mount -t overlay overlay \
            -o lowerdir=/mnt/lower,upperdir=/mnt/live/upper,workdir=/mnt/live/work \
            /mnt/root 2>/dev/null; then
            echo "[INIT] Live overlay mounted"
            if [ -x /mnt/root/sbin/init ]; then
                echo "[INIT] Found /sbin/init, executing switch_root (live CD)..."
                exec switch_root /mnt/root /sbin/init
            fi
            echo "[INIT] ERROR: /sbin/init not found in live overlay"
        else
            echo "[INIT] ERROR: overlay mount failed"
        fi
    else
        echo "[INIT] ERROR: loop/ext4 mount of nexa.img failed"
    fi
fi

echo "[INIT] ERROR: Could not mount any block device"
echo "[INIT] Available /dev entries:"
ls -la /dev/ | grep -E '^b' || true
echo "[INIT] Dropping to emergency shell..."
exec /bin/sh
INIT_EOF
chmod +x "$ROOTFS_DIR/init"

if [ "$IS_ROOTFS" = false ]; then
    echo "[*] Generating Nexa Wayland session tools..."
    mkdir -p "$MNT_DIR/usr/local/bin" "$MNT_DIR/var/log" "$MNT_DIR/run"
    cat > "$MNT_DIR/usr/local/bin/nexa-wayland-check" << 'CHECK_EOF'
#!/bin/sh
# Report the graphics/session pieces needed by the Nexa Wayland desktop.

line() {
    printf '%-18s %s\n' "$1" "$2"
}

has_cmd() {
    if command -v "$1" >/dev/null 2>&1; then
        line "$1" "OK ($(command -v "$1"))"
    else
        line "$1" "missing"
    fi
}

echo "NexaOS Wayland check"
echo "===================="
line "kernel drm" "$(ls /sys/class/drm 2>/dev/null | tr '\n' ' ')"
if [ -e /dev/dri/card0 ]; then
    line "/dev/dri/card0" "OK"
else
    line "/dev/dri/card0" "missing"
fi

if [ -d /dev/input ]; then
    line "/dev/input" "OK ($(ls /dev/input 2>/dev/null | tr '\n' ' '))"
else
    line "/dev/input" "missing"
fi

if [ -n "${XDG_RUNTIME_DIR:-}" ] && [ -d "$XDG_RUNTIME_DIR" ]; then
    line "XDG_RUNTIME_DIR" "OK ($XDG_RUNTIME_DIR)"
else
    line "XDG_RUNTIME_DIR" "missing"
fi

line "WAYLAND_DISPLAY" "${WAYLAND_DISPLAY:-unset}"
has_cmd sway
has_cmd waybar
has_cmd foot
has_cmd wofi
has_cmd seatd
has_cmd dbus-daemon

echo
echo "Start the desktop with: nexa-session"
CHECK_EOF
    chmod +x "$MNT_DIR/usr/local/bin/nexa-wayland-check"

    cat > "$MNT_DIR/usr/local/bin/nexa-session" << 'SESSION_EOF'
#!/bin/sh
# Start the Nexa Wayland desktop, with diagnostics and framebuffer fallback.

MODE="${1:-}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/1000}"
export NEXA_WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}"

# The desktop account is uid 1000, whatever the installer named it.
NEXA_USER=$(awk -F: '$3 == 1000 { print $1; exit }' /etc/passwd)

mkdir -p "$XDG_RUNTIME_DIR" /run/dbus /var/log
chmod 700 "$XDG_RUNTIME_DIR" 2>/dev/null || true
chown 1000:1000 "$XDG_RUNTIME_DIR" 2>/dev/null || true

if [ "$MODE" = "--check" ]; then
    exec /usr/local/bin/nexa-wayland-check
fi

/usr/local/bin/nexa-wayland-check || true

if command -v dbus-daemon >/dev/null 2>&1 && ! pidof dbus-daemon >/dev/null 2>&1; then
    dbus-daemon --system --fork >/var/log/dbus.log 2>&1 || true
fi

if command -v seatd >/dev/null 2>&1; then
    if [ ! -S /run/seatd.sock ]; then
        seatd -g seat >/var/log/seatd.log 2>&1 &
        for i in 1 2 3 4 5 6 7 8 9 10; do
            [ -S /run/seatd.sock ] && break
            sleep 0.2
        done
    fi
    export LIBSEAT_BACKEND=seatd
fi

if command -v sway >/dev/null 2>&1; then
    [ -x /usr/local/bin/nexa-boot-finish ] && /usr/local/bin/nexa-boot-finish
    SWAY_CMD="sway"
    if [ -f /etc/sway/config ]; then
        SWAY_CMD="sway -c /etc/sway/config"
    fi

    export WLR_BACKENDS="${WLR_BACKENDS:-drm,libinput}"
    export XDG_SESSION_TYPE=wayland
    # Force the CPU (pixman) renderer instead of GLES2/EGL. Hardware EGL
    # depends on the host's virtual GPU actually exposing 3D acceleration
    # (e.g. VirtualBox's VMSVGA driver reports "No 3D enabled" unless the
    # VM has "Enable 3D Acceleration" checked, and even then it's inconsistent
    # across hosts), and when it's unavailable sway doesn't reliably fall
    # back on its own -- it just fails to create a renderer and exits. Pixman
    # skips GPU/EGL entirely, so the desktop comes up the same way regardless
    # of the underlying virtual GPU or its acceleration state.
    export WLR_RENDERER="${WLR_RENDERER:-pixman}"

    if [ -n "$NEXA_USER" ]; then
        # waybar (and other apps) expect a session bus; without one, glib's
        # dbus autolaunch tries to use X11 and fails ("Cannot autolaunch
        # D-Bus without X11 $DISPLAY"), so give them a real one instead of
        # only the --system bus started above. dbus-run-session starts a
        # private session bus, exports DBUS_SESSION_BUS_ADDRESS, and execs
        # its argument.
        DBUS_RUN="dbus-run-session --"
        command -v dbus-run-session >/dev/null 2>&1 || DBUS_RUN=""
        exec su "$NEXA_USER" -c "unset WAYLAND_DISPLAY; export XDG_RUNTIME_DIR='$XDG_RUNTIME_DIR'; export LIBSEAT_BACKEND='${LIBSEAT_BACKEND:-}'; export WLR_BACKENDS='$WLR_BACKENDS'; export WLR_RENDERER='$WLR_RENDERER'; export XDG_SESSION_TYPE=wayland; exec $DBUS_RUN /usr/local/bin/nexa-desktop"
    fi

    unset WAYLAND_DISPLAY
    exec $SWAY_CMD
fi

echo "Sway is not installed in this image yet."
echo "Rebuild with: make disk"

if [ "$MODE" = "--wayland-only" ]; then
    exit 1
fi

if [ -x /apps/desktop-gui/run ]; then
    echo "Falling back to the Nexa framebuffer desktop."
    exec /apps/desktop-gui/run
fi

echo "No graphical fallback found; opening a shell."
exec /bin/sh -l
SESSION_EOF
    chmod +x "$MNT_DIR/usr/local/bin/nexa-session"

    # No console on tty1: it used to open a root shell there with no login
    # (anyone pressing Ctrl+Alt+F1 got root), and its text flashed up during
    # boot before the desktop. tty1 belongs to the desktop; a normal login
    # prompt for recovery is on tty2 (Ctrl+Alt+F2, password required).
    rm -f "$MNT_DIR/usr/local/bin/nexa-console"

    echo "[*] Generating Sway init service..."
    mkdir -p "$MNT_DIR/etc/init.d"
    cat > "$MNT_DIR/etc/init.d/sway" << 'SWAY_EOF'
#!/sbin/openrc-run
description="Start Nexa Wayland session"

depend() {
    need localmount
    use dbus seatd
    after devfs
}

start() {
    ebegin "Starting Nexa Wayland session"
    mkdir -p /var/log
    /usr/local/bin/nexa-session --wayland-only >/var/log/nexa-session.log 2>&1 &
    echo $! > /run/nexa-session.pid
    eend $?
}

stop() {
    ebegin "Stopping Nexa Wayland session"
    if [ -f /run/nexa-session.pid ]; then
        kill "$(cat /run/nexa-session.pid)" 2>/dev/null || true
        rm -f /run/nexa-session.pid
    fi
    pkill -f "sway" 2>/dev/null || true
    eend $?
}
SWAY_EOF
    chmod +x "$MNT_DIR/etc/init.d/sway"

    mkdir -p "$MNT_DIR/etc/runlevels/default"
    ln -sf /etc/init.d/sway "$MNT_DIR/etc/runlevels/default/sway"
    [ -x "$MNT_DIR/etc/init.d/dbus" ] && ln -sf /etc/init.d/dbus "$MNT_DIR/etc/runlevels/default/dbus"
    [ -x "$MNT_DIR/etc/init.d/seatd" ] && ln -sf /etc/init.d/seatd "$MNT_DIR/etc/runlevels/default/seatd"
    # Without a running udev, DRM/input device nodes are created by devtmpfs
    # with default root:root ownership, so sway (running as "user") can't open
    # /dev/dri/card0 (EGL_NOT_INITIALIZED / "DRI2: failed to load driver").
    # eudev's rules (lib/udev/rules.d/50-udev-default.rules) chgrp those to
    # "video", which "user" is a member of.
    [ -x "$MNT_DIR/etc/init.d/udev" ] && ln -sf /etc/init.d/udev "$MNT_DIR/etc/runlevels/default/udev"
    [ -x "$MNT_DIR/etc/init.d/udev-trigger" ] && ln -sf /etc/init.d/udev-trigger "$MNT_DIR/etc/runlevels/default/udev-trigger"
    chown -R user:user "$MNT_DIR/home/user" 2>/dev/null || true

    echo "[*] Generating network (DHCP) service..."
    # No /etc/network/interfaces + ifupdown-ng setup here on purpose: that
    # requires knowing the NIC's name ahead of time (eth0 under QEMU, but
    # enp0s3/ens18/... elsewhere), which this OS can't predict. This instead
    # DHCPs every non-loopback interface it finds at boot, so it works the
    # same whether the underlying NIC is virtio-net, e1000, or a VirtualBox
    # adapter, without hardcoding a name.
    cat > "$MNT_DIR/etc/init.d/netdhcp" << 'NETDHCP_EOF'
#!/sbin/openrc-run
description="DHCP every detected network interface (udhcpc)"

depend() {
    need localmount
    after modules
    before sshd
    provide net
}

start() {
    ebegin "Starting network interfaces (DHCP)"
    local path iface
    for path in /sys/class/net/*; do
        iface=$(basename "$path")
        case "$iface" in
            lo|sit0) continue ;;
        esac
        # Wireless interfaces belong to iwd (it runs its own DHCP once
        # associated, see /etc/iwd/main.conf); udhcpc on an unassociated
        # radio would just spin and fight iwd for the interface.
        if [ -d "$path/wireless" ] || [ -d "$path/phy80211" ]; then
            continue
        fi
        ip link set "$iface" up 2>/dev/null
        # In the background from the first moment: waiting here for a lease
        # (or for a cable that isn't plugged in) held up the whole boot by
        # ~16 s, desktop included. The address arrives a moment later.
        udhcpc -i "$iface" -b -p "/run/udhcpc.$iface.pid" -x "hostname:$(cat /etc/hostname 2>/dev/null || echo nexa)" >/dev/null 2>&1 &
    done
    eend 0
}

stop() {
    ebegin "Stopping network interfaces"
    local pf
    for pf in /run/udhcpc.*.pid; do
        [ -f "$pf" ] || continue
        kill "$(cat "$pf")" 2>/dev/null
        rm -f "$pf"
    done
    eend 0
}
NETDHCP_EOF
    chmod +x "$MNT_DIR/etc/init.d/netdhcp"
    [ -x "$MNT_DIR/etc/init.d/netdhcp" ] && ln -sf /etc/init.d/netdhcp "$MNT_DIR/etc/runlevels/default/netdhcp"

    echo "[*] Generating WiFi (iwd) configuration..."
    mkdir -p "$MNT_DIR/etc/iwd"
    # iwd does DHCP itself for WiFi links. NameResolvingService=none leaves
    # /etc/resolv.conf (provisioned statically) alone instead of expecting
    # systemd-resolved/resolvconf, neither of which this OS ships.
    cat > "$MNT_DIR/etc/iwd/main.conf" << 'IWD_CONF_EOF'
[General]
EnableNetworkConfiguration=true

[Network]
NameResolvingService=none
IWD_CONF_EOF

    # This kernel has no module support, so a built-in WiFi driver probes at
    # boot before the root filesystem (and its firmware) is mounted and gives
    # up. Re-offer every still-unbound PCI device to the driver core now that
    # /lib/firmware is reachable; harmless for devices nothing can drive.
    cat > "$MNT_DIR/etc/init.d/wifi-probe" << 'WIFIPROBE_EOF'
#!/sbin/openrc-run
description="Re-probe unbound PCI devices now that firmware is available"

depend() {
    need localmount
    after udev udev-trigger
    before iwd
}

start() {
    ebegin "Re-probing PCI devices without a driver"
    local dev
    for dev in /sys/bus/pci/devices/*; do
        [ -e "$dev/driver" ] && continue
        basename "$dev" > /sys/bus/pci/drivers_probe 2>/dev/null
    done
    eend 0
}
WIFIPROBE_EOF
    chmod +x "$MNT_DIR/etc/init.d/wifi-probe"
    ln -sf /etc/init.d/wifi-probe "$MNT_DIR/etc/runlevels/default/wifi-probe"
    [ -x "$MNT_DIR/etc/init.d/iwd" ] && ln -sf /etc/init.d/iwd "$MNT_DIR/etc/runlevels/default/iwd"

    # Bluetooth: same story as WiFi. btusb binds at boot, but the chip's
    # firmware (Intel ibt-*, Realtek rtl_bt/, ...) lives on the root
    # filesystem, so its setup fails. Unbind and re-bind each adapter once
    # /lib/firmware is there, before bluetoothd starts.
    cat > "$MNT_DIR/etc/init.d/bt-probe" << 'BTPROBE_EOF'
#!/sbin/openrc-run
description="Re-probe USB Bluetooth adapters now that firmware is available"

depend() {
    need localmount
    after udev udev-trigger
    before bluetooth
}

start() {
    ebegin "Re-probing Bluetooth adapters"
    local drv=/sys/bus/usb/drivers/btusb intf ids=""
    for intf in "$drv"/*:*.0; do
        [ -e "$intf" ] && ids="$ids ${intf##*/}"
    done
    for intf in $ids; do
        echo "$intf" > "$drv/unbind" 2>/dev/null
    done
    for intf in $ids; do
        echo "$intf" > "$drv/bind" 2>/dev/null
    done
    eend 0
}
BTPROBE_EOF
    chmod +x "$MNT_DIR/etc/init.d/bt-probe"
    ln -sf /etc/init.d/bt-probe "$MNT_DIR/etc/runlevels/default/bt-probe"
    [ -x "$MNT_DIR/etc/init.d/bluetooth" ] && ln -sf /etc/init.d/bluetooth "$MNT_DIR/etc/runlevels/default/bluetooth"

    echo "[*] Generating zram swap service..."
    # Compressed swap in RAM: the kernel has ZRAM built in (creating zram0 at
    # boot); this sizes it to half of RAM and turns it on. Doubles as a
    # cushion for small VMs without needing a swap partition on the disk.
    echo "[*] Generating Nexa Mind (on-device AI) service..."
    # nexa-mindd (Rust, tools/nexa-mindd) runs as its own unprivileged user,
    # confines itself with Landlock and starts llama-server only when asked.
    # The desktop user reaches it through /run/nexa-mind/mind.sock (group
    # nexa-mind). Models live in /var/lib/nexa-mind/models.
    cat > "$MNT_DIR/etc/init.d/nexa-mind" << 'MIND_EOF'
#!/sbin/openrc-run
description="Nexa Mind: on-device AI service"
command=/usr/local/bin/nexa-mindd
command_args="serve"
command_user="nexa-mind:nexa-mind"
command_background=true
pidfile=/run/nexa-mind.pid
output_log=/var/log/nexa-mind.log
error_log=/var/log/nexa-mind.log

depend() {
    need localmount
    after net
}

start_pre() {
    checkpath -d -m 0750 -o nexa-mind:nexa-mind /run/nexa-mind
    checkpath -d -m 0700 -o nexa-mind:nexa-mind /var/lib/nexa-mind /var/lib/nexa-mind/models
    checkpath -f -m 0640 -o nexa-mind:nexa-mind /var/log/nexa-mind.log
}
MIND_EOF
    chmod +x "$MNT_DIR/etc/init.d/nexa-mind"
    ln -sf /etc/init.d/nexa-mind "$MNT_DIR/etc/runlevels/default/nexa-mind"

    echo "[*] Generating the system broker service (nexa-brokerd)..."
    cat > "$MNT_DIR/etc/init.d/nexa-broker" << 'BROKER_EOF'
#!/sbin/openrc-run
# nexa-brokerd: the one checked, sandboxed, audited way apps change the
# system as root (tools/nexa-brokerd). Socket: /run/nexa-broker.sock.
description="NexaOS system broker"
command=/usr/local/bin/nexa-brokerd
command_args="serve"
command_background=true
pidfile=/run/nexa-broker.pid
output_log=/var/log/nexa-broker.service.log
error_log=/var/log/nexa-broker.service.log

depend() {
    need localmount
    before sway
}

start_pre() {
    checkpath -f -m 0600 -o root:root /var/log/nexa-broker.log
}
BROKER_EOF
    chmod +x "$MNT_DIR/etc/init.d/nexa-broker"
    ln -sf /etc/init.d/nexa-broker "$MNT_DIR/etc/runlevels/default/nexa-broker"

    echo "[*] Generating the boot readahead recorder (nexa-preload)..."
    # On a live boot without a readahead map (or with nexa.preload_record on
    # the kernel command line) this records which parts of nexa.img the boot
    # reads, for `make preload-map` to put on the ISO. Costs one mincore()
    # call every half second for 75 s; does nothing on installed systems.
    cat > "$MNT_DIR/etc/init.d/nexa-preload" << 'PRELOAD_EOF'
#!/sbin/openrc-run
description="Record what the live boot reads (boot readahead map)"

depend() {
    need localmount
}

start() {
    img=/mnt/nexa-live/nexa.img
    [ -f "$img" ] || return 0
    if [ -f /mnt/nexa-live/boot/nexa-preload.map ] && ! grep -qw nexa.preload_record /proc/cmdline; then
        return 0
    fi
    mkdir -p /run/nexa-preload
    start-stop-daemon --start --background --exec /usr/local/bin/nexa-preload -- record "$img" /run/nexa-preload/boot.map 75
}
PRELOAD_EOF
    chmod +x "$MNT_DIR/etc/init.d/nexa-preload"
    ln -sf /etc/init.d/nexa-preload "$MNT_DIR/etc/runlevels/default/nexa-preload"

    echo "[*] Generating the kernel link for Nexa Mind (nexa-kerneld)..."
    # nexa-kerneld (Rust, tools/nexa-kerneld): pressure (PSI triggers),
    # crashes (netlink process connector), OOM kills (/dev/kmsg) and power
    # (RAPL) for the assistant; a CPU floor (uclamp) for the app in front and
    # a cgroup that keeps the AI out of your way. Socket /run/nexa-kerneld.sock.
    cat > "$MNT_DIR/etc/init.d/nexa-kerneld" << 'KERNELD_EOF'
#!/sbin/openrc-run
description="Nexa Mind's kernel link (pressure, crashes, power, app boost)"
command=/usr/local/bin/nexa-kerneld
command_args="serve"
command_background=true
pidfile=/run/nexa-kerneld.pid
output_log=/var/log/nexa-kerneld.log
error_log=/var/log/nexa-kerneld.log

depend() {
    need localmount
    after cgroups
    before sway
}
KERNELD_EOF
    chmod +x "$MNT_DIR/etc/init.d/nexa-kerneld"
    ln -sf /etc/init.d/nexa-kerneld "$MNT_DIR/etc/runlevels/default/nexa-kerneld"

    cat > "$MNT_DIR/etc/init.d/zram" << 'ZRAM_EOF'
#!/sbin/openrc-run
description="Compressed RAM swap (zram)"

depend() {
    need localmount
    after udev
}

start() {
    ebegin "Enabling zram swap"
    if [ ! -e /sys/block/zram0 ]; then
        eend 1 "no zram device"
        return 1
    fi
    local mem_kb size_kb
    mem_kb=$(awk '/^MemTotal:/ {print $2}' /proc/meminfo)
    size_kb=$((mem_kb / 2))
    echo zstd > /sys/block/zram0/comp_algorithm 2>/dev/null
    echo "${size_kb}K" > /sys/block/zram0/disksize \
        && mkswap /dev/zram0 >/dev/null \
        && swapon -p 100 /dev/zram0
    local rc=$?
    # Swapping into compressed RAM is cheap, so prefer it over evicting page
    # cache (the kernel default of 60 assumes slow disk swap).
    echo 100 > /proc/sys/vm/swappiness 2>/dev/null
    eend $rc
}

stop() {
    ebegin "Disabling zram swap"
    swapoff /dev/zram0 2>/dev/null
    echo 1 > /sys/block/zram0/reset 2>/dev/null
    eend 0
}
ZRAM_EOF
    chmod +x "$MNT_DIR/etc/init.d/zram"
    ln -sf /etc/init.d/zram "$MNT_DIR/etc/runlevels/default/zram"

    echo "[*] System-wide GTK defaults..."
    mkdir -p "$MNT_DIR/etc/gtk-3.0"
    cat > "$MNT_DIR/etc/gtk-3.0/settings.ini" << 'GTK_EOF'
[Settings]
gtk-application-prefer-dark-theme=true
gtk-theme-name=Adwaita
gtk-icon-theme-name=Adwaita
gtk-cursor-theme-name=Adwaita
gtk-cursor-theme-size=24
gtk-font-name=Noto Sans 10
GTK_EOF

    echo "[*] Firewall and kernel hardening..."
    # Firewall: nothing gets in unless it answers a connection this machine
    # started. Outgoing traffic is unrestricted.
    cat > "$MNT_DIR/etc/nftables.nft" << 'NFT_EOF'
#!/usr/sbin/nft -f
flush ruleset

table inet filter {
    chain input {
        type filter hook input priority 0; policy drop;
        ct state established,related accept
        ct state invalid drop
        iif lo accept
        # IPv6 needs neighbour discovery to work at all.
        ip6 nexthdr icmpv6 icmpv6 type { nd-neighbor-solicit, nd-neighbor-advert, nd-router-advert, packet-too-big, destination-unreachable, time-exceeded } accept
        ip protocol icmp icmp type { destination-unreachable, time-exceeded } accept
        # DHCP replies to this machine's own requests.
        udp sport 67 udp dport 68 accept
        udp sport 547 udp dport 546 accept
    }
    chain forward {
        type filter hook forward priority 0; policy drop;
    }
    chain output {
        type filter hook output priority 0; policy accept;
    }
}
NFT_EOF
    chmod 600 "$MNT_DIR/etc/nftables.nft"
    if [ -x "$MNT_DIR/etc/init.d/nftables" ]; then
        mkdir -p "$MNT_DIR/etc/conf.d"
        echo 'rules_file="/etc/nftables.nft"' > "$MNT_DIR/etc/conf.d/nftables"
        ln -sf /etc/init.d/nftables "$MNT_DIR/etc/runlevels/default/nftables"
    fi

    # Kernel hardening, applied at boot by the sysctl service.
    mkdir -p "$MNT_DIR/etc/sysctl.d"
    cat > "$MNT_DIR/etc/sysctl.d/90-nexa-hardening.conf" << 'SYSCTL_EOF'
# Hide kernel addresses and the kernel log from unprivileged users.
kernel.kptr_restrict = 2
kernel.dmesg_restrict = 1
kernel.yama.ptrace_scope = 1
kernel.unprivileged_bpf_disabled = 1
kernel.kexec_load_disabled = 1
# Safer handling of links and files in shared directories like /tmp.
fs.protected_symlinks = 1
fs.protected_hardlinks = 1
fs.protected_fifos = 2
fs.protected_regular = 2
fs.suid_dumpable = 0
# Network: drop spoofed and redirected traffic, resist SYN floods.
net.ipv4.conf.all.rp_filter = 1
net.ipv4.conf.default.rp_filter = 1
net.ipv4.conf.all.accept_redirects = 0
net.ipv4.conf.default.accept_redirects = 0
net.ipv4.conf.all.send_redirects = 0
net.ipv4.conf.all.accept_source_route = 0
net.ipv6.conf.all.accept_redirects = 0
net.ipv6.conf.default.accept_redirects = 0
net.ipv4.tcp_syncookies = 1
net.ipv4.icmp_echo_ignore_broadcasts = 1
SYSCTL_EOF
    mkdir -p "$MNT_DIR/etc/runlevels/boot"
    [ -x "$MNT_DIR/etc/init.d/sysctl" ] && ln -sf /etc/init.d/sysctl "$MNT_DIR/etc/runlevels/boot/sysctl"
    # cgroup v2 at /sys/fs/cgroup: nexa-kerneld gives the AI its own CPU share.
    [ -x "$MNT_DIR/etc/init.d/cgroups" ] && ln -sf /etc/init.d/cgroups "$MNT_DIR/etc/runlevels/boot/cgroups"
    [ -f "$MNT_DIR/etc/rc.conf" ] && sed -i 's/^#\?rc_cgroup_mode=.*/rc_cgroup_mode="unified"/' "$MNT_DIR/etc/rc.conf"
    # Start services side by side (each still waits for what it needs): the
    # desktop no longer waits behind Wi-Fi, Bluetooth, the firewall or the AI.
    [ -f "$MNT_DIR/etc/rc.conf" ] && sed -i 's/^#\?rc_parallel=.*/rc_parallel="YES"/' "$MNT_DIR/etc/rc.conf"

    echo "[*] Enabling syslog and crond (sshd stays off)..."
    # All three are already installed (openssh/busybox-extras from
    # ALPINE_PACKAGES_CORE/UTILS) but --no-scripts skips the apk triggers
    # that would normally enable them, so without this they sit unused.
    # sshd generates its own host keys on first start (see its init script's
    # generate_host_keys()), uniquely per machine -- never baked in at build
    # time, which would make every NexaOS image share the same keys.
    [ -x "$MNT_DIR/etc/init.d/syslog" ] && ln -sf /etc/init.d/syslog "$MNT_DIR/etc/runlevels/default/syslog"
    # sshd is installed but NOT started by default: a desktop shouldn't accept
    # remote password logins out of the box. Enable with: rc-update add sshd
    rm -f "$MNT_DIR/etc/runlevels/default/sshd"
    [ -x "$MNT_DIR/etc/init.d/crond" ] && ln -sf /etc/init.d/crond "$MNT_DIR/etc/runlevels/default/crond"
    # /etc/acpi/PWRF/00000080 (from the acpid package defaults) already runs
    # `poweroff` on a power-button event -- this just starts the daemon that
    # listens for it, so VirtualBox/QEMU/host "power off" acts as a clean
    # shutdown instead of a hard kill.
    [ -x "$MNT_DIR/etc/init.d/acpid" ] && ln -sf /etc/init.d/acpid "$MNT_DIR/etc/runlevels/default/acpid"

    echo "[*] Generating boot splash service..."
    cat > "$MNT_DIR/usr/local/bin/nexa-boot-progress" << 'PROGRESS_EOF'
#!/bin/sh
# Time-estimated progress for the boot animation (bootsplash --daemon, started
# by the initramfs): there's no clean "% of boot done" signal in OpenRC's
# dependency graph, so this climbs from where the initramfs left off to 90%
# with stage names, and holds; nexa-boot-finish jumps it to 100%.
PCT=40
while [ $PCT -lt 90 ]; do
    if [ $PCT -lt 55 ]; then MSG="Starting services..."
    elif [ $PCT -lt 72 ]; then MSG="Bringing up the network..."
    else MSG="Preparing your desktop..."
    fi
    bootsplash $PCT "$MSG"
    sleep 0.4
    PCT=$((PCT + 3))
done
bootsplash 90 "Preparing your desktop..."
PROGRESS_EOF
    chmod +x "$MNT_DIR/usr/local/bin/nexa-boot-progress"

    cat > "$MNT_DIR/usr/local/bin/nexa-boot-finish" << 'FINISH_EOF'
#!/bin/sh
# Called right before sway takes the display: close the progress ring, fade
# out, and wait until the animation has released the screen.
[ -f /run/bootsplash.pid ] && kill "$(cat /run/bootsplash.pid)" 2>/dev/null
rm -f /run/bootsplash.pid
bootsplash 100 "Welcome"
bootsplash --quit
FINISH_EOF
    chmod +x "$MNT_DIR/usr/local/bin/nexa-boot-finish"

    cat > "$MNT_DIR/etc/init.d/bootsplash" << 'BOOTSPLASH_EOF'
#!/sbin/openrc-run
description="NexaOS boot animation progress"

depend() {
    before *
}

start() {
    ebegin "Starting boot animation progress"
    if [ -x /usr/local/bin/bootsplash ] && grep -qw splash /proc/cmdline; then
        # Normally already running since the initramfs; start one if not.
        bootsplash --ping || { bootsplash --daemon </dev/null >/dev/null 2>&1 & }
        /usr/local/bin/nexa-boot-progress </dev/null >/dev/null 2>&1 &
        echo $! > /run/bootsplash.pid
    fi
    eend 0
}

stop() {
    [ -f /run/bootsplash.pid ] && kill "$(cat /run/bootsplash.pid)" 2>/dev/null
    rm -f /run/bootsplash.pid
    eend 0
}
BOOTSPLASH_EOF
    chmod +x "$MNT_DIR/etc/init.d/bootsplash"
    [ -x "$MNT_DIR/etc/init.d/bootsplash" ] && ln -sf /etc/init.d/bootsplash "$MNT_DIR/etc/runlevels/default/bootsplash"

    echo "[*] Generating inittab..."
    cat > "$MNT_DIR/etc/inittab" << 'INITTAB_EOF'
::sysinit:/sbin/openrc sysinit
::sysinit:/sbin/openrc boot
::wait:/sbin/openrc default
::wait:/usr/local/bin/bootsplash --quit
::ctrlaltdel:/sbin/openrc reboot
::shutdown:/sbin/openrc shutdown
tty2::respawn:/sbin/getty 38400 tty2
ttyS0::respawn:/sbin/getty -L ttyS0 115200 vt100
INITTAB_EOF

    echo "[*] Generating Sway configuration..."
    mkdir -p "$MNT_DIR/etc/sway" "$MNT_DIR/home/user/.config/sway/config.d"
    printf '# Per-user sway settings (managed by the Settings app) go in this directory.\n' \
        > "$MNT_DIR/home/user/.config/sway/config.d/00-nexa.conf"
    # Numeric ids: the Settings app (uid 1000) must be able to write here.
    chown -R 1000:1000 "$MNT_DIR/home/user/.config"
    cat > "$MNT_DIR/etc/sway/config" << 'SWAY_CONFIG_EOF'
# NexaOS Sway Configuration
set $mod Mod4
set $term foot
set $menu nexa-start
set $assistant nexa-ai-gui
set $files nexa-files
set $browser nexa-browser

# Fonts
font pango:monospace 10

# Gaps & Borders
smart_gaps on
gaps inner 5
gaps outer 5
default_border pixel 2
# Apps that leave the title bar to the compositor get sway's (themed) bar,
# so every window has something to drag; apps that draw their own (all Nexa
# apps, foot) are unaffected.
default_floating_border normal 2

# Drag any window (not just its titlebar) by holding mod and clicking.
floating_modifier $mod

# Windows open floating/overlapping by default (a normal desktop feel)
# instead of auto-tiling into side-by-side regions. mod+shift+space still
# toggles a window into the tiled layer for anyone who wants that instead.
for_window [app_id=".*"] floating enable

# Output configuration
output * bg /usr/share/backgrounds/nexaos/wallpaper.png fill

# Cursor: without a theme, GTK and sway disagree and the pointer changes
# size and style between the desktop and apps.
seat * xcursor_theme Adwaita 24

# Laptop touchpads: tap to click, natural scrolling, ignore while typing.
# Tap-and-drag (tap, then touch again and slide) moves windows and files;
# drag lock keeps the drag going when the finger lifts at the touchpad's
# edge, so a window crosses the screen on a small laptop touchpad.
input type:touchpad {
    tap enabled
    drag enabled
    drag_lock enabled
    natural_scroll enabled
    dwt enabled
    middle_emulation enabled
}

# Closing the lid locks the screen and suspends (see nexa-lid).
bindswitch --reload --locked lid:on exec nexa-lid

# Startup applications. The lock comes first so an installed system shows
# the lock screen before anything else (see nexa-login-lock).
exec nexa-login-lock
exec pipewire
exec pipewire-pulse
exec wireplumber
# Render the saved theme (bar, notifications, terminal, borders) before the
# bar and notification daemon read their config.
exec sh -c 'nexa-theme apply --saved --no-reload >/dev/null 2>&1; swaymsg reload >/dev/null 2>&1; waybar & exec mako'
exec nexa-dock
exec nexa-idle
exec nexa-battery-watch
# Tell nexa-kerneld which app is in front (it gets a CPU floor).
exec nexa-kerneld focus-watch
# Nexa Patterns (off until switched on in Settings > Privacy) and its desktop card.
exec nexa-patternd serve
exec nexa-foryou

# Keybindings
bindsym $mod+Return exec $term
bindsym $mod+d exec $menu
bindsym $mod+space exec nexa-nexus
bindsym $mod+n exec nexa-control
bindsym $mod+a exec $assistant
bindsym $mod+e exec $files
bindsym $mod+w exec $browser
bindsym $mod+Shift+c kill
bindsym $mod+Shift+e exec nexa-power
# Emergency exit in case the power menu itself is broken
bindsym $mod+Ctrl+Shift+e exit
bindsym $mod+Shift+r reload

# Screenshot (grim/slurp, see /usr/local/bin/nexa-screenshot)
bindsym Print exec nexa-screenshot
bindsym Shift+Print exec nexa-screenshot --select
bindsym Ctrl+Shift+Escape exec nexa-pulse
bindsym XF86AudioRaiseVolume exec wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+
bindsym XF86AudioLowerVolume exec wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-
bindsym XF86AudioMute exec wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle
bindsym --locked XF86MonBrightnessUp exec brightnessctl set 5%+
bindsym --locked XF86MonBrightnessDown exec brightnessctl set 5%-

# Lock screen (see /usr/local/bin/nexa-lock)
bindsym $mod+Shift+l exec nexa-lock

# Workspace names
set $ws1 "1"
set $ws2 "2"
set $ws3 "3"
set $ws4 "4"
set $ws5 "5"

# Switch to workspace
bindsym $mod+1 workspace $ws1
bindsym $mod+2 workspace $ws2
bindsym $mod+3 workspace $ws3
bindsym $mod+4 workspace $ws4
bindsym $mod+5 workspace $ws5

# Move window to workspace
bindsym $mod+Shift+1 move container to workspace $ws1
bindsym $mod+Shift+2 move container to workspace $ws2
bindsym $mod+Shift+3 move container to workspace $ws3
bindsym $mod+Shift+4 move container to workspace $ws4
bindsym $mod+Shift+5 move container to workspace $ws5

# Navigation
bindsym $mod+Left focus left
bindsym $mod+Down focus down
bindsym $mod+Up focus up
bindsym $mod+Right focus right

# Resize
bindsym $mod+Shift+Left resize shrink width 10px
bindsym $mod+Shift+Down resize grow height 10px
bindsym $mod+Shift+Up resize shrink height 10px
bindsym $mod+Shift+Right resize grow width 10px

# Floating
bindsym $mod+shift+space floating toggle
# Super+Space is Nexus, so the tiled/floating focus toggle moves here.
bindsym $mod+Ctrl+space focus mode_toggle

# Cycle focus between open windows, alt-tab style
bindsym $mod+Tab focus next
bindsym $mod+Shift+Tab focus prev

# Scratchpad (restores windows minimized via each app's titlebar button --
# sway ignores Wayland's own minimize request since there's no taskbar to
# restore from, so minimize moves windows here instead; the waybar
# sway/scratchpad module below also restores on click)
bindsym $mod+minus move scratchpad
bindsym $mod+Shift+minus scratchpad show

# Per-user settings written by the Settings app (e.g. keyboard layout). A stub
# file is always shipped so the wildcard is guaranteed to match something.
include ~/.config/sway/config.d/*.conf

# Taskbar
# Waybar is a standalone layer-shell bar (launched via `exec waybar` above,
# configured in /etc/xdg/waybar/config) — it is not an i3bar status_command,
# so no `bar { status_command ... }` block here (that pairing doesn't speak
# the same protocol and just prints "error reading from status command").
SWAY_CONFIG_EOF

    echo "[*] Generating Hyprland session (GPU machines)..."
    mkdir -p "$MNT_DIR/etc/hypr"
    cat > "$MNT_DIR/etc/hypr/hyprland.conf" << 'HYPR_EOF'
# NexaOS Hyprland session: animations, blur, rounded corners and shadows.
# Colours come from the active theme (nexa-theme writes the sourced file).
monitor = , preferred, auto, 1
source = ~/.config/hypr/nexa-theme.conf

env = XCURSOR_THEME,Adwaita
env = XCURSOR_SIZE,24

exec-once = nexa-login-lock
exec-once = pipewire
exec-once = pipewire-pulse
exec-once = wireplumber
exec-once = swaybg -i /usr/share/backgrounds/nexaos/wallpaper.png -m fill
exec-once = waybar -c /etc/xdg/waybar/config-hyprland
exec-once = mako
exec-once = nexa-dock
exec-once = nexa-idle
exec-once = nexa-battery-watch
exec-once = nexa-kerneld focus-watch
exec-once = nexa-patternd serve
exec-once = nexa-foryou
exec-once = nexa-keyboard-layout

general {
    gaps_in = 6
    gaps_out = 12
    border_size = 2
    layout = dwindle
    resize_on_border = true
}

decoration {
    rounding = 12
    active_opacity = 1.0
    inactive_opacity = 0.97
    blur {
        enabled = true
        size = 8
        passes = 3
        vibrancy = 0.17
    }
    shadow {
        enabled = true
        range = 24
        render_power = 3
        color = rgba(00000073)
    }
}

# macOS-like motion: fast start, long soft landing.
animations {
    enabled = true
    bezier = nexa, 0.22, 1, 0.36, 1
    bezier = nexaOut, 0.55, 0, 1, 0.45
    animation = windowsIn, 1, 5, nexa, popin 80%
    animation = windowsOut, 1, 4, nexaOut, popin 80%
    animation = windowsMove, 1, 5, nexa
    animation = fade, 1, 5, nexa
    animation = border, 1, 8, nexa
    animation = workspaces, 1, 6, nexa, slide
    animation = specialWorkspace, 1, 5, nexa, slidevert
    animation = layers, 1, 4, nexa, fade
}

input {
    kb_layout = us
    follow_mouse = 1
    touchpad {
        natural_scroll = true
        tap-to-click = true
        tap-and-drag = true
        drag_lock = 1
        disable_while_typing = true
    }
}

# Three fingers sideways switches workspaces, like a Mac trackpad.
gesture = 3, horizontal, workspace

misc {
    disable_hyprland_logo = true
    disable_splash_rendering = true
    # No warning banners across the top of the screen at every login:
    # NexaOS starts Hyprland through start-hyprland (nexa-desktop) and sets
    # the desktop environment itself; hyprland-guiutils (optional dialogs)
    # isn't packaged by Alpine 3.24.
    disable_watchdog_warning = true
    disable_xdg_env_checks = true
    disable_hyprland_guiutils_check = true
    force_default_wallpaper = 0
    focus_on_activate = true
}

# Normal-desktop feel: windows float and open centred (same as the sway session).
windowrule = float on, match:class .*
windowrule = center on, match:class .*

# Frosted glass behind the bar, the Dock and the panels; the panels animate
# themselves, so Hyprland's own layer animation is off for them.
layerrule = blur on, match:namespace ^(waybar|nexa-.*)$
layerrule = ignore_alpha 0.1, match:namespace ^(waybar|nexa-.*)$
layerrule = no_anim on, match:namespace ^(nexa-panel)$

bind = SUPER, Return, exec, foot
bind = SUPER, D, exec, nexa-start
bind = SUPER, Space, exec, nexa-nexus
bind = SUPER, N, exec, nexa-control
bind = SUPER, A, exec, nexa-ai-gui
bind = SUPER, E, exec, nexa-files
bind = SUPER, W, exec, nexa-browser
bind = SUPER SHIFT, C, killactive
bind = SUPER SHIFT, E, exec, nexa-power
bind = SUPER CTRL SHIFT, E, exit
bind = SUPER SHIFT, R, exec, hyprctl reload
bind = SUPER SHIFT, L, exec, nexa-lock
bind = SUPER, M, fullscreen, 1
bind = SUPER, F, fullscreen, 0
bind = SUPER SHIFT, Space, togglefloating
bind = , Print, exec, nexa-screenshot
bind = SHIFT, Print, exec, nexa-screenshot --select
bind = CTRL SHIFT, Escape, exec, nexa-pulse
bind = SUPER, Left, movefocus, l
bind = SUPER, Right, movefocus, r
bind = SUPER, Up, movefocus, u
bind = SUPER, Down, movefocus, d
bind = SUPER, Tab, cyclenext
bind = SUPER, Tab, bringactivetotop
bind = SUPER, 1, workspace, 1
bind = SUPER, 2, workspace, 2
bind = SUPER, 3, workspace, 3
bind = SUPER, 4, workspace, 4
bind = SUPER, 5, workspace, 5
bind = SUPER SHIFT, 1, movetoworkspace, 1
bind = SUPER SHIFT, 2, movetoworkspace, 2
bind = SUPER SHIFT, 3, movetoworkspace, 3
bind = SUPER SHIFT, 4, movetoworkspace, 4
bind = SUPER SHIFT, 5, movetoworkspace, 5
bindm = SUPER, mouse:272, movewindow
# Alt + drag anywhere in a window moves it too (needs no title bar).
bindm = ALT, mouse:272, movewindow
bindm = SUPER, mouse:273, resizewindow
bindel = , XF86AudioRaiseVolume, exec, wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+
bindel = , XF86AudioLowerVolume, exec, wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-
bindl = , XF86AudioMute, exec, wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle
bindel = , XF86MonBrightnessUp, exec, brightnessctl set 5%+
bindel = , XF86MonBrightnessDown, exec, brightnessctl set 5%-
bindl = , switch:on:Lid Switch, exec, nexa-lid
HYPR_EOF

    # Starts the user's desktop: Hyprland where a real GPU driver is loaded
    # (animations, blur), Sway otherwise, or if Hyprland fails to come up.
    cat > "$MNT_DIR/usr/local/bin/nexa-desktop" << 'DESKTOP_SESSION_EOF'
#!/bin/sh
# Override with one word, "sway" or "hyprland", in ~/.config/nexa/session
# or /etc/nexa/session.
choice=$(cat "$HOME/.config/nexa/session" /etc/nexa/session 2>/dev/null | head -n 1)

# Never a desktop inside a desktop: started from within a running session
# (anything that runs "nexa-desktop" by mistake), this would open a second
# full session in a window, with its own lock screen.
if [ -n "${WAYLAND_DISPLAY:-}" ] || [ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ] || [ -n "${SWAYSOCK:-}" ]; then
    echo "nexa-desktop: already inside a desktop session; not starting another" >&2
    exit 1
fi

gpu_ok() {
    for drv in /sys/class/drm/card[0-9]*/device/driver; do
        case "$(basename "$(readlink "$drv" 2>/dev/null)")" in
            i915|xe|amdgpu|radeon|nouveau) return 0 ;;
        esac
    done
    return 1
}

start_sway() {
    export WLR_RENDERER="${WLR_RENDERER:-pixman}"
    exec sway -c /etc/sway/config
}

nexa-theme apply --saved --no-reload >/dev/null 2>&1
case "$choice" in
    sway) start_sway ;;
    hyprland) ;;
    *) { gpu_ok && command -v start-hyprland >/dev/null 2>&1; } || start_sway ;;
esac

mkdir -p "$HOME/.cache"
started=$(date +%s)
start-hyprland -- -c /etc/hypr/hyprland.conf >"$HOME/.cache/nexa-hyprland.log" 2>&1
# Exiting within seconds means Hyprland couldn't use this GPU: fall back.
if [ $(( $(date +%s) - started )) -lt 20 ]; then
    start_sway
fi
DESKTOP_SESSION_EOF
    chmod +x "$MNT_DIR/usr/local/bin/nexa-desktop"

    echo "[*] Generating Waybar configuration..."
    mkdir -p "$MNT_DIR/etc/xdg/waybar"
    cat > "$MNT_DIR/etc/xdg/waybar/config" << 'WAYBAR_CONFIG_EOF'
{
    "layer": "top",
    "position": "top",
    "height": 38,
    "modules-left": ["custom/start", "sway/workspaces", "sway/mode", "sway/scratchpad"],
    "modules-center": ["clock"],
    "modules-right": ["network", "backlight", "pulseaudio", "battery", "custom/notify", "custom/control"],

    "custom/start": {
        "format": "",
        "tooltip-format": "Start (Super+D)",
        "on-click": "nexa-start"
    },

    "sway/workspaces": {
        "disable-scroll": true,
        "all-outputs": true
    },

    "sway/scratchpad": {
        "format": "minimized ({count})",
        "format-empty": "",
        "tooltip": true,
        "tooltip-format": "{app}: {title}"
    },

    "clock": {
        "format": "{:%a %d %b   %H:%M}",
        "tooltip-format": "{:%A %d %B %Y}",
        "interval": 30,
        "on-click": "nexa-control"
    },

    "network": {
        "format-wifi": "  {essid}",
        "format-ethernet": "  Wired",
        "format-disconnected": "  Offline",
        "tooltip-format-wifi": "{essid} · {signalStrength}%",
        "on-click": "nexa-network"
    },

    "backlight": {
        "format": "  {percent}%",
        "on-scroll-up": "brightnessctl set 5%+",
        "on-scroll-down": "brightnessctl set 5%-",
        "on-click": "nexa-control"
    },

    "pulseaudio": {
        "format": "{icon}  {volume}%",
        "format-muted": "  Muted",
        "format-icons": {"default": ["", ""]},
        "on-click": "nexa-control",
        "on-click-right": "nexa-volume"
    },

    "battery": {
        "states": {"warning": 25, "critical": 10},
        "format": "{icon}  {capacity}%",
        "format-charging": "  {capacity}%",
        "format-plugged": "  {capacity}%",
        "format-icons": ["", "", "", "", ""],
        "tooltip-format": "{timeTo}",
        "on-click": "nexa-control"
    },

    "custom/notify": {
        "exec": "nexa-bar-notify",
        "return-type": "json",
        "interval": 3,
        "on-click": "nexa-control"
    },

    "custom/control": {
        "format": "",
        "tooltip-format": "Control Centre (Super+N)",
        "on-click": "nexa-control"
    }
}
WAYBAR_CONFIG_EOF

    sed -e 's#"sway/workspaces", "sway/mode", "sway/scratchpad"#"hyprland/workspaces"#' \
        -e 's#"sway/workspaces": {#"hyprland/workspaces": {#' \
        "$MNT_DIR/etc/xdg/waybar/config" > "$MNT_DIR/etc/xdg/waybar/config-hyprland"

    echo "[*] Generating Mako (notification daemon) configuration..."
    mkdir -p "$MNT_DIR/etc/xdg/mako"
    cat > "$MNT_DIR/etc/xdg/mako/config" << 'MAKO_CONFIG_EOF'
background-color=#232323ee
text-color=#e0e0e0
border-color=#7aa2f7
border-size=2
border-radius=10
padding=12
margin=10
default-timeout=6000
max-visible=5
font=monospace 10
anchor=top-right

[urgency=low]
border-color=#3a3a3a

[urgency=critical]
border-color=#f7768e
default-timeout=0
MAKO_CONFIG_EOF

fi

echo "[✓] Configuration files generated"
