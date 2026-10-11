#!/bin/sh
# Build a bootable hybrid ISO: BIOS via isolinux, UEFI via a standalone GRUB
# EFI binary, bootable as an optical disc (VirtualBox, real CD/DVD) or from a
# USB stick written with Rufus in either mode:
#   - DD mode: the raw ISO lands on the stick; the isohybrid MBR (BIOS) and
#     the GPT-flagged EFI image (UEFI) make the stick itself bootable.
#   - ISO mode: Rufus copies the files onto FAT32 and installs syslinux from
#     /boot/isolinux (BIOS); UEFI firmware runs /EFI/BOOT/BOOTX64.EFI.
# Every entry passes nexa.live so /init goes straight to looking for nexa.img
# on whatever medium this ended up on (see build-initramfs.sh).
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

apk add --no-cache xorriso syslinux grub grub-efi mtools dosfstools >/dev/null

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT

mkdir -p "$STAGE/boot/isolinux"
cp "$DIST_DIR/bzImage" "$STAGE/boot/vmlinuz"
cp "$DIST_DIR/initramfs.cpio.gz" "$STAGE/boot/initramfs.cpio.gz"
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
TIMEOUT 50
PROMPT 1

LABEL nexa
  MENU LABEL NexaOS (live)
  KERNEL /boot/vmlinuz
  APPEND initrd=/boot/initramfs.cpio.gz nexa.live console=tty0 console=ttyS0
CFG_EOF

# UEFI. The volume label can't be used to find the boot files: Rufus's ISO
# mode relabels the stick. A marker file works on the CD and on any copy.
# The device GRUB was loaded from is checked first (the CD itself, or the
# FAT partition in Rufus ISO mode); a full search is only the fallback, for
# Rufus DD mode where GRUB comes from the small EFI image instead.
touch "$STAGE/boot/nexa-live.id"
mkdir -p "$STAGE/boot/grub" "$STAGE/EFI/BOOT"
cat > "$STAGE/boot/grub/grub.cfg" << 'CFG_EOF'
set timeout=5
set default=0
insmod all_video

menuentry "NexaOS (live)" {
    linux /boot/vmlinuz nexa.live console=tty0 console=ttyS0
    initrd /boot/initramfs.cpio.gz
}
CFG_EOF
cat > "$STAGE/embedded.cfg" << 'CFG_EOF'
echo "NexaOS: loaded from $cmdpath"
regexp --set=1:bootdev '^\(([^)]*)\)' "$cmdpath"
if [ -n "$bootdev" -a -f "($bootdev)/boot/nexa-live.id" ]; then
    set root=$bootdev
else
    search --no-floppy --file /boot/nexa-live.id --set=root
fi
echo "NexaOS: boot files on ($root)"
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
        efi_gop efi_uga video_bochs video_cirrus gzio ls" \
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
