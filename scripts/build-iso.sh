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
# usb-storage.delay_use=200ms: the kernel waits 1 s before reading a new
# USB stick by default (for old sticks that need settling); that second
# was spent at every live boot waiting for the stick NexaOS runs from.
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

# Boot readahead map (make preload-map). It lists places inside nexa.img, so
# it only fits the nexa.img it was recorded on: the boot skips any other, and
# a map left over from an earlier build once shipped that way (3.2.0 booted
# with no readahead). Checked first, before any slow work: a stale map stops
# the build; a missing one only warns.
MAP="$DIST_DIR/nexa-preload.map"
if [ -f "$MAP" ]; then
    PRELOAD=
    for p in build/native/nexa-preload rootfs/usr/local/bin/nexa-preload; do
        [ -x "$p" ] && { PRELOAD=$p; break; }
    done
    if [ -z "$PRELOAD" ]; then
        echo "ERROR: can't check $MAP: nexa-preload isn't built (run 'make native')." >&2
        exit 1
    fi
    if ! "$PRELOAD" matches "$DIST_DIR/nexa.img" "$MAP" 2>/dev/null; then
        cat >&2 <<EOF
ERROR: $MAP was recorded for a different nexa.img (an earlier build).
  Every boot would skip it, so this ISO would have no boot readahead.
  Run 'make preload-map': it packs the ISO without the old map, boots it
  once to record a new one, and packs the ISO again with that.
EOF
        exit 1
    fi
else
    echo "[!] No boot readahead map: this ISO boots without readahead. Run 'make preload-map' before giving it out."
fi

# Retried: a DNS hiccup in the container makes apk report the packages as
# missing ("no such package"), which used to fail the whole build.
# Already in the nexa-build image: only missing tools are downloaded.
missing=$(for p in xorriso syslinux grub grub-efi mtools dosfstools sbsigntool; do apk info -e "$p" >/dev/null || echo "$p"; done)
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
# Boot readahead map, checked against nexa.img at the top.
[ -f "$MAP" ] && cp "$MAP" "$STAGE/boot/nexa-preload.map"
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
  APPEND initrd=/boot/initramfs-core.cpio.gz,/boot/initramfs-i915.cpio,/boot/initramfs-amdgpu.cpio,/boot/initramfs-iwlwifi.cpio nexa.live mac80211_hwsim.radios=0 usb-storage.delay_use=200ms quiet splash loglevel=3 vt.global_cursor_default=0 console=ttyS0 console=tty0

LABEL fresh
  MENU LABEL NexaOS (live, don't load saved changes)
  KERNEL /boot/vmlinuz
  APPEND initrd=/boot/initramfs-core.cpio.gz,/boot/initramfs-i915.cpio,/boot/initramfs-amdgpu.cpio,/boot/initramfs-iwlwifi.cpio nexa.live nexa.nopersist mac80211_hwsim.radios=0 usb-storage.delay_use=200ms quiet splash loglevel=3 vt.global_cursor_default=0 console=ttyS0 console=tty0
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
    linux /boot/vmlinuz nexa.live mac80211_hwsim.radios=0 usb-storage.delay_use=200ms quiet splash loglevel=3 vt.global_cursor_default=0 console=ttyS0 console=tty0
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
    linux /boot/vmlinuz nexa.live mac80211_hwsim.radios=0 usb-storage.delay_use=200ms quiet splash loglevel=3 vt.global_cursor_default=0 console=ttyS0 console=tty0
    nexa_initrd
}
# Persistent live USB: start once without the files and settings kept on
# the stick (e.g. if a saved change stops the desktop from starting).
menuentry "NexaOS (live, don't load saved changes)" {
    linux /boot/vmlinuz nexa.live nexa.nopersist mac80211_hwsim.radios=0 usb-storage.delay_use=200ms quiet splash loglevel=3 vt.global_cursor_default=0 console=ttyS0 console=tty0
    nexa_initrd
}
# Skips the i915/amdgpu drivers and stays on the firmware framebuffer.
menuentry "NexaOS (live, safe graphics)" {
    linux /boot/vmlinuz nexa.live mac80211_hwsim.radios=0 usb-storage.delay_use=200ms nomodeset quiet splash loglevel=3 vt.global_cursor_default=0 console=ttyS0 console=tty0
    nexa_initrd
}
# Kernel messages on screen from the first instant (EFI framebuffer
# earlycon), nothing filtered, and a panic stays on screen.
menuentry "NexaOS (live, debug: show all boot messages)" {
    echo "Loading kernel..."
    linux /boot/vmlinuz nexa.live mac80211_hwsim.radios=0 usb-storage.delay_use=200ms earlycon=efifb keep_bootcon ignore_loglevel panic=0 console=ttyS0 console=tty0
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
# GRUB's built-in first script runs before its full language is loaded:
# plain commands only (no if/else). --hint looks on the device GRUB was
# started from first, so another NexaOS stick or disk isn't picked by mistake.
cat > "$STAGE/embedded.cfg" << 'CFG_EOF'
regexp --set=1:bootdev '^\(([^)]*)\)' "$cmdpath"
search --no-floppy --file /boot/nexa-live.id --set=root --hint="$bootdev"
configfile ($root)/boot/grub/grub.cfg
CFG_EOF
# One GRUB image with every module it needs built in (`grub-mkimage`, not
# a standalone image with modules in a memdisk): with Secure Boot on, GRUB
# refuses to load module files, so nothing may need loading. Only the
# modules this boot uses: with all of them in, GRUB probes every
# filesystem type it knows on the CD, and under UEFI on an optical drive
# that hangs (seen in QEMU/OVMF: `ls (cd0)/` never returned). The SBAT
# section says which GRUB this is; shim refuses a GRUB without one, or
# with a generation older than the revocations (grub,5 since 2025).
cat > "$STAGE/sbat.csv" << 'SBAT_EOF'
sbat,1,SBAT Version,sbat,1,https://github.com/rhboot/shim/blob/main/SBAT.md
grub,5,Free Software Foundation,grub,2.14,https://www.gnu.org/software/grub/
grub.nexaos,1,NexaOS,grub,2.14,https://github.com/B-less/NexaOS
SBAT_EOF
grub-mkimage -O x86_64-efi -o "$STAGE/grub.efi" -p /boot/grub -c "$STAGE/embedded.cfg" --sbat "$STAGE/sbat.csv" \
    normal linux echo test regexp search search_fs_file configfile iso9660 fat exfat ntfs part_gpt part_msdos \
    all_video efi_gop efi_uga video_bochs video_cirrus gzio ls reboot halt efifwsetup sleep smbios keystatus
rm "$STAGE/embedded.cfg" "$STAGE/sbat.csv"

# Secure Boot (docs/SECURE_BOOT.md): with the NexaOS key on the build
# machine (scripts/secureboot-key.sh, mounted at $NEXA_SB_KEYS) and the
# Microsoft-signed shim fetched (scripts/fetch-shim.sh -> build/shim), the
# firmware starts shim, shim starts GRUB (signed with the NexaOS key),
# GRUB starts the kernel (signed too, checked through shim). A PC learns
# the NexaOS key once, in MokManager, from ENROLL-THIS-KEY-NexaOS.cer on
# the stick. Without them: the same GRUB, unsigned (Secure Boot off).
SB_KEYS=${NEXA_SB_KEYS:-}
EFI_FILES="BOOTX64.EFI"
if [ -n "$SB_KEYS" ] && [ -f "$SB_KEYS/nexa-sb.key" ] && [ -f "$SB_KEYS/nexa-sb.crt" ] && [ -f "$SB_KEYS/nexa-sb.cer" ] \
        && [ -f build/shim/shimx64.efi ] && [ -f build/shim/mmx64.efi ]; then
    sbsign --key "$SB_KEYS/nexa-sb.key" --cert "$SB_KEYS/nexa-sb.crt" --output "$STAGE/EFI/BOOT/grubx64.efi" "$STAGE/grub.efi" >/dev/null
    sbsign --key "$SB_KEYS/nexa-sb.key" --cert "$SB_KEYS/nexa-sb.crt" --output "$STAGE/boot/vmlinuz.signed" "$STAGE/boot/vmlinuz" >/dev/null
    mv "$STAGE/boot/vmlinuz.signed" "$STAGE/boot/vmlinuz"
    cp build/shim/shimx64.efi "$STAGE/EFI/BOOT/BOOTX64.EFI"
    cp build/shim/mmx64.efi "$STAGE/EFI/BOOT/mmx64.efi"
    cp "$SB_KEYS/nexa-sb.cer" "$STAGE/EFI/BOOT/ENROLL-THIS-KEY-NexaOS.cer"
    cp "$SB_KEYS/nexa-sb.cer" "$STAGE/ENROLL-THIS-KEY-NexaOS.cer"
    cat > "$STAGE/SECURE-BOOT-README.txt" << 'SBREADME_EOF'
NexaOS starts with Secure Boot ON.

The first time on each PC, a blue screen ("Shim UEFI key management")
appears instead of NexaOS. Do this once:

  1. Press a key, choose "Enroll key from disk".
  2. Choose this USB stick, then the file ENROLL-THIS-KEY-NexaOS.cer
     (inside EFI/BOOT if it isn't at the top).
  3. Choose "Continue", then "Yes", then "Reboot".

From then on this PC starts NexaOS with Secure Boot left on.

If the PC refuses the stick before that screen ("Secure Boot violation",
or the boot menu just returns), its firmware doesn't trust Microsoft's
third-party UEFI CA yet. Turn that on; it doesn't turn Secure Boot off:
  ThinkPad: F1 > Security > Secure Boot > "Allow Microsoft 3rd Party UEFI CA"
  Other PCs: "Third-party UEFI CA", "Microsoft UEFI CA" or similar.
Or just start the stick with Secure Boot off.

  NEVER choose "Clear All Secure Boot Keys", "Delete All Keys" or
  "Reset to Setup Mode". That removes Microsoft's Windows key too, and
  Windows then won't start (the boot menu just blinks and returns).

If that already happened: in the same menu choose "Restore Factory Keys"
("Reset to Default Keys", "Install Default Keys"), save and restart.
Have your BitLocker recovery key ready (aka.ms/myrecoverykey): Windows
may ask for it once. If Windows still won't start with Secure Boot on,
turn Secure Boot off, start Windows, install all updates, then turn
Secure Boot on again.

NexaOS checks this at every start and warns you on the desktop
(details: nexa-doctor firmware-keys --text in a terminal).
SBREADME_EOF
    EFI_FILES="BOOTX64.EFI grubx64.efi mmx64.efi ENROLL-THIS-KEY-NexaOS.cer"
    echo "[*] Secure Boot: shim (Microsoft-signed) + GRUB and kernel signed with the NexaOS key"
else
    mv "$STAGE/grub.efi" "$STAGE/EFI/BOOT/BOOTX64.EFI"
    echo "[!] Secure Boot: no NexaOS key or shim here (scripts/secureboot-key.sh, scripts/fetch-shim.sh); this ISO needs Secure Boot off."
fi
rm -f "$STAGE/grub.efi"

# El Torito UEFI boot image: a small FAT filesystem holding the same binary.
EFI_IMG="$STAGE/boot/efiboot.img"
EFI_KB=$(( $(du -kc "$STAGE/EFI/BOOT"/* | tail -1 | cut -f1) + 1024 ))
mkfs.vfat -C "$EFI_IMG" "$EFI_KB" >/dev/null
mmd -i "$EFI_IMG" ::/EFI ::/EFI/BOOT
for f in $EFI_FILES; do
    mcopy -i "$EFI_IMG" "$STAGE/EFI/BOOT/$f" "::/EFI/BOOT/$f"
done
# MokManager browses this small partition on a stick written in DD mode.
[ -f "$STAGE/ENROLL-THIS-KEY-NexaOS.cer" ] && mcopy -i "$EFI_IMG" "$STAGE/ENROLL-THIS-KEY-NexaOS.cer" ::/ENROLL-THIS-KEY-NexaOS.cer

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
