#!/bin/sh
# Build a bootable hybrid ISO: BIOS via isolinux, UEFI via a standalone GRUB
# EFI binary, bootable as an optical disc (VirtualBox, real CD/DVD) or from a
# USB stick written with Rufus in either mode:
#   - DD mode: the raw ISO lands on the stick; the isohybrid MBR (BIOS) and
#     the GPT-flagged EFI image (UEFI) make the stick itself bootable.
#   - ISO mode: Rufus copies the files onto FAT32 and installs syslinux from
#     /boot/isolinux (BIOS); UEFI firmware runs /EFI/BOOT/BOOTX64.EFI.
# Every entry passes nexa.live so /init goes straight to looking for nexa.img
# on whatever medium this ended up on (see build-initramfs.sh), and
# mac80211_hwsim.radios=0: the kernel has the virtual Wi-Fi simulator built
# in for VM testing, and its fake wlan0 otherwise shadows the real card.
#
# It ships the exact same kernel, initramfs, and rootfs (nexa.img) as a
# direct-disk boot, just with nexa.img embedded as a plain file on the ISO
# instead of attached as a virtio-blk/SATA disk. /init's live-CD fallback
# (see build-initramfs.sh) finds it there, loop-mounts it read-only, and
# layers a tmpfs on top via overlayfs -- a normal writable-feeling session
# that simply discards changes on reboot, like any other live CD.
#
# Runs as root inside a throwaway Alpine container (same no-host-sudo
# pattern as the rest of the build): xorriso/syslinux are build-time-only
# tools, never installed in the shipped OS.
set -eu

DIST_DIR=$1
ISO_OUTPUT=$2

# Retried: a DNS hiccup in the container makes apk report the packages as
# missing ("no such package"), which used to fail the whole build.
# Already in the nexa-build image: only missing tools are downloaded.
missing=$(for p in xorriso syslinux grub grub-efi mtools dosfstools; do apk info -e "$p" >/dev/null || echo "$p"; done)
for try in 1 2 3 4 5; do
    [ -z "$missing" ] && break
    apk add --no-cache $missing >/dev/null 2>&1 && break
    [ "$try" = 5 ] && { echo "ERROR: couldn't download the ISO tools (no internet in the build container?)"; exit 1; }
    echo "[*] package download failed (network?), retrying in 5 s ($try/5)"
    sleep 5
done

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT

mkdir -p "$STAGE/boot/isolinux"
cp "$DIST_DIR/bzImage" "$STAGE/boot/vmlinuz"
# The initramfs in three pieces (see build-initramfs.sh) so the boot loader
# reads only the GPU firmware this machine can use.
for f in initramfs-core.cpio.gz initramfs-i915.cpio initramfs-amdgpu.cpio initramfs-iwlwifi.cpio; do
    cp "$DIST_DIR/$f" "$STAGE/boot/$f"
done
# Boot readahead map (make preload-map); ignored at boot if it was made for
# another build of nexa.img.
[ -f "$DIST_DIR/nexa-preload.map" ] && cp "$DIST_DIR/nexa-preload.map" "$STAGE/boot/nexa-preload.map"
cp "$DIST_DIR/nexa.img" "$STAGE/nexa.img"

for f in isolinux.bin ldlinux.c32; do
    src=""
    for d in /usr/share/syslinux /usr/lib/syslinux/bios; do
        [ -f "$d/$f" ] && src="$d/$f" && break
    done
    if [ -z "$src" ]; then
        echo "ERROR: $f not found (syslinux package layout changed?)" >&2
        exit 1
    fi
    cp "$src" "$STAGE/boot/isolinux/$f"
done

cat > "$STAGE/boot/isolinux/isolinux.cfg" << 'CFG_EOF'
DEFAULT nexa
TIMEOUT 10
PROMPT 1

LABEL nexa
  MENU LABEL NexaOS (live)
  KERNEL /boot/vmlinuz
  APPEND initrd=/boot/initramfs-core.cpio.gz,/boot/initramfs-i915.cpio,/boot/initramfs-amdgpu.cpio,/boot/initramfs-iwlwifi.cpio nexa.live mac80211_hwsim.radios=0 quiet splash loglevel=3 vt.global_cursor_default=0 console=ttyS0 console=tty0

LABEL fresh
  MENU LABEL NexaOS (live, don't load saved changes)
  KERNEL /boot/vmlinuz
  APPEND initrd=/boot/initramfs-core.cpio.gz,/boot/initramfs-i915.cpio,/boot/initramfs-amdgpu.cpio,/boot/initramfs-iwlwifi.cpio nexa.live nexa.nopersist mac80211_hwsim.radios=0 quiet splash loglevel=3 vt.global_cursor_default=0 console=ttyS0 console=tty0
CFG_EOF

# UEFI. The volume label can't be used to find the boot files: Rufus's ISO
# mode relabels the stick. A marker file works on the CD and on any copy.
# The device GRUB was loaded from is checked first (the CD itself, or the
# FAT partition in Rufus ISO mode); a full search is only the fallback, for
# Rufus DD mode where GRUB comes from the small EFI image instead.
touch "$STAGE/boot/nexa-live.id"
mkdir -p "$STAGE/boot/grub" "$STAGE/EFI/BOOT"
cat > "$STAGE/boot/grub/grub.cfg" << 'CFG_EOF'
insmod all_video
# GPU firmware: Intel's with an Intel CPU, AMD's with an AMD one (read from
# the firmware's SMBIOS tables), both when that can't be told. Intel Wi-Fi
# firmware always: AMD laptops often have Intel Wi-Fi cards too.
set nexa_gpu=both
if smbios --type 4 --get-string 7 --set nexa_cpu; then
    if regexp Intel "$nexa_cpu"; then set nexa_gpu=intel; fi
    if regexp "AMD|Advanced Micro" "$nexa_cpu"; then set nexa_gpu=amd; fi
fi
export nexa_gpu
function nexa_initrd {
    if [ "$nexa_gpu" = intel ]; then
        initrd /boot/initramfs-core.cpio.gz /boot/initramfs-i915.cpio /boot/initramfs-iwlwifi.cpio
    elif [ "$nexa_gpu" = amd ]; then
        initrd /boot/initramfs-core.cpio.gz /boot/initramfs-amdgpu.cpio /boot/initramfs-iwlwifi.cpio
    else
        initrd /boot/initramfs-core.cpio.gz /boot/initramfs-i915.cpio /boot/initramfs-amdgpu.cpio /boot/initramfs-iwlwifi.cpio
    fi
}
# Boot straight away, no menu and no waiting (the boot animation takes the
# screen next); hold Shift for the menu. If the firmware can't report keys,
# Esc within 1 s shows it instead.
set nexa_menu=
if keystatus; then
    if keystatus --shift; then set nexa_menu=1; fi
else
    if sleep --interruptible 1; then set nexa_menu=; else set nexa_menu=1; fi
fi
if [ -z "$nexa_menu" ]; then
    linux /boot/vmlinuz nexa.live mac80211_hwsim.radios=0 quiet splash loglevel=3 vt.global_cursor_default=0 console=ttyS0 console=tty0
    nexa_initrd
    boot
fi
set timeout=-1
set default=0
set color_normal=white/blue
set color_highlight=black/light-gray
set menu_color_normal=white/blue
set menu_color_highlight=black/light-gray

menuentry "NexaOS (live)" {
    linux /boot/vmlinuz nexa.live mac80211_hwsim.radios=0 quiet splash loglevel=3 vt.global_cursor_default=0 console=ttyS0 console=tty0
    nexa_initrd
}
# Persistent live USB: start once without the files and settings kept on
# the stick (e.g. if a saved change stops the desktop from starting).
menuentry "NexaOS (live, don't load saved changes)" {
    linux /boot/vmlinuz nexa.live nexa.nopersist mac80211_hwsim.radios=0 quiet splash loglevel=3 vt.global_cursor_default=0 console=ttyS0 console=tty0
    nexa_initrd
}
# Skips the i915/amdgpu drivers and stays on the firmware framebuffer.
menuentry "NexaOS (live, safe graphics)" {
    linux /boot/vmlinuz nexa.live mac80211_hwsim.radios=0 nomodeset quiet splash loglevel=3 vt.global_cursor_default=0 console=ttyS0 console=tty0
    nexa_initrd
}
# Kernel messages on screen from the first instant (EFI framebuffer
# earlycon), nothing filtered, and a panic stays on screen.
menuentry "NexaOS (live, debug: show all boot messages)" {
    echo "Loading kernel..."
    linux /boot/vmlinuz nexa.live mac80211_hwsim.radios=0 earlycon=efifb keep_bootcon ignore_loglevel panic=0 console=ttyS0 console=tty0
    echo "Loading initramfs..."
    nexa_initrd
    echo "Starting kernel..."
}
menuentry "UEFI firmware settings" {
    fwsetup
}
menuentry "Reboot" {
    reboot
}
CFG_EOF
cat > "$STAGE/embedded.cfg" << 'CFG_EOF'
regexp --set=1:bootdev '^\(([^)]*)\)' "$cmdpath"
if [ -n "$bootdev" -a -f "($bootdev)/boot/nexa-live.id" ]; then
    set root=$bootdev
else
    search --no-floppy --file /boot/nexa-live.id --set=root
fi
configfile ($root)/boot/grub/grub.cfg
CFG_EOF
# prefix is deliberately left at GRUB's built-in memdisk: that's where the
# packed modules are, and commands like linux/initrd auto-load from it.
# Standalone: the modules are packed into the one binary, so it needs
# nothing else from the medium except grub.cfg. Only the modules this boot
# uses: with all of them packed in, GRUB probes every filesystem type it
# knows on the CD, and under UEFI on an optical drive that hangs (seen in
# QEMU/OVMF: `ls (cd0)/` never returned).
grub-mkstandalone -O x86_64-efi -o "$STAGE/EFI/BOOT/BOOTX64.EFI" \
    --install-modules="normal linux echo test regexp search search_fs_file \
        configfile iso9660 fat exfat ntfs part_gpt part_msdos all_video \
        efi_gop efi_uga video_bochs video_cirrus gzio ls reboot halt efifwsetup sleep smbios keystatus" \
    --modules="part_gpt part_msdos iso9660 fat" \
    --locales="" --fonts="" --themes="" \
    "boot/grub/grub.cfg=$STAGE/embedded.cfg"
rm "$STAGE/embedded.cfg"

# El Torito UEFI boot image: a small FAT filesystem holding the same binary.
EFI_IMG="$STAGE/boot/efiboot.img"
EFI_KB=$(( $(du -k "$STAGE/EFI/BOOT/BOOTX64.EFI" | cut -f1) + 1024 ))
mkfs.vfat -C "$EFI_IMG" "$EFI_KB" >/dev/null
mmd -i "$EFI_IMG" ::/EFI ::/EFI/BOOT
mcopy -i "$EFI_IMG" "$STAGE/EFI/BOOT/BOOTX64.EFI" ::/EFI/BOOT/BOOTX64.EFI

ISOHDPFX=""
for d in /usr/share/syslinux /usr/lib/syslinux/bios; do
    [ -f "$d/isohdpfx.bin" ] && ISOHDPFX="$d/isohdpfx.bin" && break
done
if [ -z "$ISOHDPFX" ]; then
    echo "ERROR: isohdpfx.bin not found (syslinux package layout changed?)" >&2
    exit 1
fi

mkdir -p "$(dirname "$ISO_OUTPUT")"
rm -f "$ISO_OUTPUT"
xorriso -as mkisofs \
    -V "NEXAOS" \
    -J -joliet-long -r \
    -isohybrid-mbr "$ISOHDPFX" \
    -b boot/isolinux/isolinux.bin \
    -c boot/isolinux/boot.cat \
    -no-emul-boot -boot-load-size 4 -boot-info-table \
    -eltorito-alt-boot \
    -e boot/efiboot.img \
    -no-emul-boot -isohybrid-gpt-basdat \
    -o "$ISO_OUTPUT" \
    "$STAGE"

echo "[OK] ISO created: $ISO_OUTPUT ($(du -h "$ISO_OUTPUT" | cut -f1))"
