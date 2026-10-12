#!/bin/sh
# Provision a NexaOS rootfs with the Wayland/Sway desktop stack and identity files.
#
# Must run as root (this script is invoked inside a throwaway Alpine container by
# the Makefile, so the build never needs host `sudo`/chroot/mount). apk installs
# leave some files root-owned/unreadable by design (e.g. busybox-suid's bbsuid),
# so identity files are (re)written *after* package installation, never before —
# reinstalling alpine-base/alpine-baselayout resets /etc/passwd, /etc/group,
# /etc/hostname and /etc/fstab to their package defaults.
set -eu

ROOTFS_DIR=$1
PKG_CORE=$2
PKG_WAYLAND=$3
PKG_SWAY=$4
PKG_TERM=$5
PKG_UTILS=$6
DNS_PRIMARY=$7
DNS_SECONDARY=$8

echo "[*] apk update"
apk update --root "$ROOTFS_DIR"

echo "[*] installing core packages"
apk add --root "$ROOTFS_DIR" --no-scripts --no-cache $PKG_CORE

echo "[*] installing Wayland packages"
apk add --root "$ROOTFS_DIR" --no-scripts --no-cache $PKG_WAYLAND

echo "[*] installing Sway/Waybar/Wofi stack"
apk add --root "$ROOTFS_DIR" --no-scripts --no-cache $PKG_SWAY

echo "[*] installing terminal and utilities"
apk add --root "$ROOTFS_DIR" --no-scripts --no-cache $PKG_TERM $PKG_UTILS

echo "[*] verifying Wayland stack is installed"
for bin in sway waybar foot wofi seatd dbus-daemon; do
    if ! [ -x "$ROOTFS_DIR/usr/bin/$bin" ] && ! [ -x "$ROOTFS_DIR/usr/sbin/$bin" ] && ! [ -x "$ROOTFS_DIR/bin/$bin" ] && ! [ -x "$ROOTFS_DIR/sbin/$bin" ]; then
        echo "ERROR: $bin missing after package installation" >&2
        exit 1
    fi
done

echo "[*] rebuilding GTK/gdk-pixbuf caches (--no-scripts skips these triggers)"
# --no-scripts is what makes apk installs reproducible without host root/chroot
# (no maintainer scripts touching /proc, /dev, users, ...), but it also skips
# the handful of *pure filesystem* cache-building triggers GTK apps need at
# runtime (wofi otherwise crashes: "Failed to load .../image-missing.png:
# Unrecognized image file format"). Those are safe to run via chroot since
# they only read/write files under the target root, no /proc or /dev needed.
chroot "$ROOTFS_DIR" /usr/bin/gdk-pixbuf-query-loaders --update-cache 2>/dev/null || true
chroot "$ROOTFS_DIR" /usr/bin/glib-compile-schemas /usr/share/glib-2.0/schemas 2>/dev/null || true
[ -x "$ROOTFS_DIR/usr/bin/gtk-update-icon-cache" ] && [ -d "$ROOTFS_DIR/usr/share/icons/hicolor" ] && \
    chroot "$ROOTFS_DIR" /usr/bin/gtk-update-icon-cache -f -t /usr/share/icons/hicolor 2>/dev/null || true
# shared-mime-info's own mime.cache trigger is skipped too. Without it, GIO's
# content-type sniffing can't recognize PNG bytes at all (even though the
# built-in PNG loader itself is present and correctly linked against libpng),
# so every icon lookup that falls through to gdk-pixbuf's built-in
# "image-missing" fallback dies with "Unrecognized image file format" (traced
# by hand: gdk-pixbuf-query-loaders alone was NOT enough, this was the actual
# root cause of the icon crashes worked around in nexa_ai/gui.py).
[ -x "$ROOTFS_DIR/usr/bin/update-mime-database" ] && [ -d "$ROOTFS_DIR/usr/share/mime" ] && \
    chroot "$ROOTFS_DIR" /usr/bin/update-mime-database /usr/share/mime 2>/dev/null || true

echo "[*] writing identity and base config (post-install, so apk cannot clobber it)"
echo "nexa" > "$ROOTFS_DIR/etc/hostname"
# A time zone must exist: without /etc/localtime, waybar's clock module can't
# resolve local time and hides itself (the bar's centre was empty). UTC by
# default; the installer and Settings change it.
ln -sf /usr/share/zoneinfo/UTC "$ROOTFS_DIR/etc/localtime"
echo "UTC" > "$ROOTFS_DIR/etc/timezone"
printf 'nameserver %s\nnameserver %s\n' "$DNS_PRIMARY" "$DNS_SECONDARY" > "$ROOTFS_DIR/etc/resolv.conf"
printf 'proc /proc proc defaults 0 0\nsysfs /sys sysfs defaults 0 0\ndevtmpfs /dev devtmpfs defaults 0 0\ntmpfs /tmp tmpfs defaults,nodev,nosuid,noexec,relatime,size=1G 0 0\ntmpfs /run tmpfs defaults,nodev,nosuid,noexec,relatime,size=1G 0 0\n' > "$ROOTFS_DIR/etc/fstab"
mkdir -p "$ROOTFS_DIR/etc/profile.d" "$ROOTFS_DIR/root" "$ROOTFS_DIR/home/user"
printf 'root:x:0:0:root:/root:/bin/sh\nmessagebus:x:81:81:messagebus:/var/run/dbus:/sbin/nologin\nseatd:x:86:86:seat daemon:/run/seatd:/sbin/nologin\nsshd:x:22:22:sshd privsep:/var/empty:/sbin/nologin\nuser:x:1000:1000:user:/home/user:/bin/sh\n' > "$ROOTFS_DIR/etc/passwd"
printf 'root::0:0:99999:7:::\nmessagebus:!:0:0:99999:7:::\nseatd:!:0:0:99999:7:::\nsshd:!:0:0:99999:7:::\nuser::0:0:99999:7:::\n' > "$ROOTFS_DIR/etc/shadow"
printf 'root:x:0:\nmessagebus:x:81:\nseat:x:86:user\nseatd:x:86:\nsshd:x:22:\nuser:x:1000:\nwheel:x:10:user\nvideo:x:27:user\naudio:x:18:user\ninput:x:97:user\n' > "$ROOTFS_DIR/etc/group"
mkdir -p "$ROOTFS_DIR/var/empty"
# sshd's privilege-separation chroot requires this owned by root and not
# group/world-writable -- explicit numeric chown because Docker's bind-mount
# ownership translation can otherwise leave a freshly-mkdir'd directory owned
# by the host UID instead of root (same reason "user"'s files use numeric
# 1000:1000 below rather than a name lookup).
chown 0:0 "$ROOTFS_DIR/var/empty"
chmod 755 "$ROOTFS_DIR/var/empty"
# /etc/passwd and /etc/group must stay world-readable: daemons that drop
# privileges (dbus-daemon runs as messagebus) look users up through them, and
# with mode 600 the system bus silently refuses every non-root client -- which
# breaks iwd/WiFi control from the desktop account. Only shadow is secret.
# Numeric root ownership for the same reason as /var/empty above: files
# written here can otherwise show up owned by the host user's uid.
chown 0:0 "$ROOTFS_DIR/etc/passwd" "$ROOTFS_DIR/etc/group" "$ROOTFS_DIR/etc/shadow"
chmod 644 "$ROOTFS_DIR/etc/passwd" "$ROOTFS_DIR/etc/group"
chmod 600 "$ROOTFS_DIR/etc/shadow"
printf 'export XDG_RUNTIME_DIR=/run/user/1000\nexport WAYLAND_DISPLAY=wayland-0\nexport QT_QPA_PLATFORM=wayland\nexport EGL_PLATFORM=wayland\nexport GDK_BACKEND=wayland\n' > "$ROOTFS_DIR/etc/profile.d/wayland.sh"
chmod 644 "$ROOTFS_DIR/etc/profile.d/wayland.sh"
chown -R 1000:1000 "$ROOTFS_DIR/home/user" 2>/dev/null || true

echo "[*] staging boot splash binary"
mkdir -p "$ROOTFS_DIR/usr/local/bin"
cp build/bootsplash_bin "$ROOTFS_DIR/usr/local/bin/bootsplash"
chmod +x "$ROOTFS_DIR/usr/local/bin/bootsplash"

echo "[*] staging app icons into the hicolor icon theme"
mkdir -p "$ROOTFS_DIR/usr/share/icons/hicolor/128x128/apps" "$ROOTFS_DIR/usr/share/icons/hicolor/scalable/apps"
cp build/icons/*.png "$ROOTFS_DIR/usr/share/icons/hicolor/128x128/apps/"
cp assets/icons/*.svg "$ROOTFS_DIR/usr/share/icons/hicolor/scalable/apps/"
# Must re-run AFTER these icons land: the icon-theme.cache built above (before
# these files existed) is stale and would otherwise make GTK ignore the new
# icons on disk entirely, falling back to the "missing image" placeholder.
[ -x "$ROOTFS_DIR/usr/bin/gtk-update-icon-cache" ] && \
    chroot "$ROOTFS_DIR" /usr/bin/gtk-update-icon-cache -f -t /usr/share/icons/hicolor 2>/dev/null || true

echo "[*] staging Nexa GUI apps and generating Sway/Waybar/OpenRC configs"
NEXAOS_BASE="$(pwd)" sh scripts/stage-gui-rootfs.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-nexa-ai.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-file-manager.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-text-editor.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-music-player.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-calculator.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-settings.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-desktop-icons.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-screenshot.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-browser.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-lock.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-viewer.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-archive.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-taskmgr.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-sysmon.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-pulse.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-pdf.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-chars.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-clipboard.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-colorpicker.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-notes.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-software.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-installer.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-volume.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-network.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-power.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-automount.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-setup.sh "$ROOTFS_DIR"
NEXAOS_BASE="$(pwd)" sh scripts/stage-shell.sh "$ROOTFS_DIR"

# Re-run now that every stage-*.sh script (including stage-installer.sh,
# which adds /usr/share/mime/packages/nexa-package.xml for the *.nexa MIME
# type) has actually staged its files -- the earlier update-mime-database
# call above ran before any of this existed, same ordering rule as the icon
# caches.
[ -x "$ROOTFS_DIR/usr/bin/update-mime-database" ] && [ -d "$ROOTFS_DIR/usr/share/mime" ] && \
    chroot "$ROOTFS_DIR" /usr/bin/update-mime-database /usr/share/mime 2>/dev/null || true

# Rebuilds /usr/share/applications/mimeinfo.cache (MIME type -> .desktop file
# mapping), same "must run after every .desktop file is staged" rule as the
# icon caches above -- GIO's Gio.AppInfo.get_default_for_type()/
# get_all_for_type() (used by the file manager's Open/Open With) read this
# cache instead of scanning every .desktop file at runtime.
[ -x "$ROOTFS_DIR/usr/bin/update-desktop-database" ] && \
    chroot "$ROOTFS_DIR" /usr/bin/update-desktop-database /usr/share/applications 2>/dev/null || true

bash scripts/generate-configs.sh "$ROOTFS_DIR" "$ROOTFS_DIR"

echo "[OK] rootfs provisioned: $(du -sh "$ROOTFS_DIR" | cut -f1)"
