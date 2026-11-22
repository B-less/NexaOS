#!/bin/sh
# Proves the live USB's GRUB verifies the kernel (the fix for Alpine GRUB
# having no shim_lock): builds a pgp-enforcing GRUB EFI image exactly as
# build-iso.sh does -- embedded public key, check_signatures=enforce, detached
# signatures on grub.cfg and the kernel -- then boots it in QEMU/OVMF twice:
#
#   A) kernel signed correctly  -> GRUB boots it (serial shows "Linux version")
#   B) one byte flipped          -> GRUB refuses it ("bad signature", no boot)
#
# Independent of Secure Boot (pgp verification runs with SB on or off), so no
# MOK/db setup is needed. Local tool: needs qemu, OVMF, grub-mkimage (with the
# x86_64-efi pgp module), mtools, gpg and a built kernel in dist/. Skips
# cleanly (exit 0) when any of those is missing, as CI has none of them.
# Usage: sh scripts/test-grub-verify.sh
set -eu
REPO=$(cd "$(dirname "$0")/.." && pwd)

skip() { echo "SKIP: $*"; exit 0; }
need() { command -v "$1" >/dev/null 2>&1 || skip "$1 not installed"; }
need qemu-system-x86_64; need grub-mkimage; need gpg; need mformat; need mcopy

KERNEL="$REPO/dist/bzImage"
INITRD="$REPO/dist/initramfs-core.cpio.gz"
[ -f "$KERNEL" ] || skip "no kernel at dist/bzImage (build it first)"
[ -f "$INITRD" ] || skip "no dist/initramfs-core.cpio.gz (build it first)"
[ -f /usr/lib/grub/x86_64-efi/pgp.mod ] || skip "this grub has no x86_64-efi pgp module"
OVMF=""
for c in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd /usr/share/ovmf/OVMF.fd /usr/share/qemu/OVMF.fd; do
    [ -f "$c" ] && { OVMF="$c"; break; }
done
[ -n "$OVMF" ] || skip "no OVMF firmware found"
VARS=""
for c in /usr/share/OVMF/OVMF_VARS_4M.fd /usr/share/OVMF/OVMF_VARS.fd; do [ -f "$c" ] && { VARS="$c"; break; }; done
ACCEL="-machine q35"; [ -r /dev/kvm ] && [ -w /dev/kvm ] && ACCEL="-machine q35 -enable-kvm"

W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
mkdir -p "$W/esp/EFI/BOOT" "$W/esp/boot/grub" "$W/gnupg"; chmod 700 "$W/gnupg"
export GNUPGHOME="$W/gnupg"
LASTLOG=""
fail() { echo "FAIL: $*"; [ -n "$LASTLOG" ] && [ -f "$LASTLOG" ] && { echo "--- last serial (tail) ---"; tail -25 "$LASTLOG"; }; exit 1; }

# 1. a throwaway verification key, exactly the shape grub-verify-key.sh makes.
cat > "$W/params" <<EOF
%no-protection
Key-Type: RSA
Key-Length: 3072
Key-Usage: sign
Name-Real: NexaOS GRUB Verify Test
Expire-Date: 0
%commit
EOF
gpg --batch --quiet --gen-key "$W/params" 2>/dev/null
KEYID=$(gpg --list-keys --with-colons | awk -F: '/^pub:/{print $5; exit}')
gpg --batch --export "$KEYID" > "$W/pubkey.gpg"

# 2. stage the files GRUB loads, and sign them.
cp "$KERNEL" "$W/esp/boot/vmlinuz"
cp "$INITRD" "$W/esp/boot/initramfs.cpio.gz"
: > "$W/esp/boot/nexa-live.id"
cat > "$W/esp/boot/grub/grub.cfg" <<'CFG'
echo "GRUBVERIFY-CFG-OK"
linux /boot/vmlinuz console=ttyS0 earlyprintk=serial,ttyS0 ignore_loglevel panic=1
initrd /boot/initramfs.cpio.gz
boot
CFG
sign() { gpg --batch --yes --detach-sign --local-user "$KEYID" -o "$1.sig" "$1"; }
sign "$W/esp/boot/grub/grub.cfg"; sign "$W/esp/boot/vmlinuz"; sign "$W/esp/boot/initramfs.cpio.gz"

# 3. the pgp-enforcing GRUB image (same recipe as build-iso.sh).
cat > "$W/embedded.cfg" <<'EMB'
serial --unit=0 --speed=115200
terminal_input serial console
terminal_output serial console
search --no-floppy --file /boot/nexa-live.id --set=root
set check_signatures=enforce
export check_signatures
configfile ($root)/boot/grub/grub.cfg
EMB
grub-mkimage -O x86_64-efi -o "$W/esp/EFI/BOOT/BOOTX64.EFI" -p /boot/grub \
    --pubkey "$W/pubkey.gpg" -c "$W/embedded.cfg" \
    normal linux echo test regexp search search_fs_file configfile fat part_gpt part_msdos \
    all_video efi_gop gzio ls reboot halt sleep serial pgp gcry_rsa gcry_sha256 gcry_sha512

run_qemu() {  # -> serial log on stdout
    rm -f "$W/esp.img"
    sz=$(du -sm "$W/esp" | cut -f1); sz=$((sz + 20))
    dd if=/dev/zero of="$W/esp.img" bs=1M count=$sz status=none
    MTOOLS_SKIP_CHECK=1 mformat -i "$W/esp.img" -F ::
    MTOOLS_SKIP_CHECK=1 mcopy -s -i "$W/esp.img" "$W/esp"/EFI "$W/esp"/boot ::
    PF="-drive if=pflash,format=raw,readonly=on,file=$OVMF"
    [ -n "$VARS" ] && { cp "$VARS" "$W/vars.fd"; PF="$PF -drive if=pflash,format=raw,file=$W/vars.fd"; }
    timeout 90 qemu-system-x86_64 $ACCEL -m 2G $PF \
        -drive format=raw,file="$W/esp.img" -nographic -serial mon:stdio -display none -no-reboot 2>&1 || true
}

echo "[*] case A: correctly signed kernel should boot"
run_qemu > "$W/a.log" 2>&1; LASTLOG="$W/a.log"
grep -aq "GRUBVERIFY-CFG-OK" "$W/a.log" || fail "GRUB never ran the signed grub.cfg (case A)"
grep -aq "Linux version" "$W/a.log" || fail "signed kernel did not boot (case A) -- see $W/a.log"
grep -aqi "bad signature" "$W/a.log" && fail "unexpected signature error on the untampered kernel (case A)"
echo "ok  A: signed grub.cfg ran and the signed kernel booted"

echo "[*] case B: a tampered kernel must be refused"
printf 'X' | dd of="$W/esp/boot/vmlinuz" bs=1 seek=4096 count=1 conv=notrunc status=none
run_qemu > "$W/b.log" 2>&1; LASTLOG="$W/b.log"
grep -aqi "bad signature" "$W/b.log" || fail "GRUB did NOT report a bad signature on the tampered kernel (case B) -- see $W/b.log"
grep -aq "Linux version" "$W/b.log" && fail "tampered kernel BOOTED -- verification is not enforced (case B)"
echo "ok  B: the tampered kernel was refused (bad signature) and never booted"

echo PASS
