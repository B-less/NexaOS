#!/bin/sh
# Windows Boot Repair (nexa-bootfix) end to end, in QEMU:
#   a fake Windows disk (GPT: EFI partition with a shim standing in for
#   bootmgfw.efi, BCD, NTFS with \Windows) as an internal NVMe disk, plus a
#   FAT32 USB stick for the report. Inside the live ISO it breaks the boot
#   manager, removes the fallback loader (no Windows boot entry exists
#   either), runs `nexa-bootfix` step by step answering "yes", and checks
#   that the files were put back, the entry written, and the report, backup
#   and screenshot saved on the stick.
#   A second NVMe disk holds Windows with user folders; inside the ISO its
#   partition list is erased (partitions deleted, as on a real ThinkPad).
#   nexa-bootfix must find the partitions, copy files to the stick
#   (not AppData) and leave that disk byte-for-byte unchanged. Once picking
#   in Files (run as the desktop user, on the read-only drive), once by
#   whole folders.
#   sh scripts/test-bootfix.sh      (needs dist/NexaOS-*.iso and build/shim)
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
work=${TMPDIR:-/tmp}/nexa-bootfix-test
mkdir -p "$work/serve"
[ -f "$repo/build/shim/shimx64.efi" ] || sh "$repo/scripts/fetch-shim.sh"
cp "$repo/build/shim/shimx64.efi" "$work/"
docker run --rm -v "$work:/w" -w /w alpine:3.24 sh -c '
set -e
apk add -q sgdisk dosfstools mtools ntfs-3g-progs sfdisk >/dev/null
truncate -s 0 win.img usb.img lost.img
truncate -s 1G win.img
sgdisk -o -n1:2048:+100M -t1:ef00 -c1:"EFI system partition" -n2:0:+16M -t2:0c01 -c2:"Microsoft reserved partition" -n3:0:0 -t3:0700 -c3:"Basic data partition" win.img >/dev/null
first() { sgdisk -i"$1" win.img | sed -n "s/First sector: \([0-9]*\).*/\1/p"; }
last() { sgdisk -i"$1" win.img | sed -n "s/Last sector: \([0-9]*\).*/\1/p"; }
s1=$(first 1); e1=$(last 1); s3=$(first 3); e3=$(last 3)
mkfs.vfat -F32 -n SYSTEM --offset "$s1" win.img $(( (e1-s1+1)/2 )) >/dev/null 2>&1
export MTOOLS_SKIP_CHECK=1
M="win.img@@$((s1*512))"
mmd -i "$M" ::/EFI ::/EFI/Microsoft ::/EFI/Microsoft/Boot ::/EFI/Boot
mcopy -i "$M" shimx64.efi ::/EFI/Microsoft/Boot/bootmgfw.efi
mcopy -i "$M" shimx64.efi ::/EFI/Boot/bootx64.efi
printf regf > /tmp/BCD; head -c 32764 /dev/zero >> /tmp/BCD; mcopy -i "$M" /tmp/BCD ::/EFI/Microsoft/Boot/BCD
truncate -s $(( (e3-s3+1)*512 )) /tmp/ntfs.img
mkntfs -q -F -f -L Windows /tmp/ntfs.img >/dev/null 2>&1
dd if=/tmp/ntfs.img of=win.img bs=512 seek="$s3" conv=notrunc status=none
truncate -s 512M lost.img
# Windows installed in Legacy BIOS mode, its partition list emptied (an
# empty GPT, as the Windows installer leaves it): System Reserved (boot
# files) at 1 MiB, C: at 101 MiB, recovery at 501 MiB.
sgdisk -o lost.img >/dev/null
ntfs_at() { truncate -s 0 /tmp/p.img; truncate -s $(( $2 * 512 )) /tmp/p.img
    mkntfs -q -F -f -p "$1" -H 255 -S 63 -L "$3" /tmp/p.img >/dev/null 2>&1
    dd if=/tmp/p.img of=lost.img bs=512 seek="$1" conv=notrunc status=none; }
ntfs_at 2048 204800 "System Reserved"
ntfs_at 206848 819200 Windows
ntfs_at 1026048 18432 Recovery
truncate -s 64M usb.img
echo type=c | sfdisk -q usb.img
mkfs.vfat -F32 -n REPORTS --offset 2048 usb.img $(( (131072-2048)/2 )) >/dev/null 2>&1
chmod 666 win.img usb.img lost.img'
# Its boot partition: a stand-in for Windows' boot code that says on the
# serial port it was started (the BIOS -> MBR -> active partition path), a
# bootmgr, and a BCD naming the three partitions on disk 1234ABCD.
python3 - "$work" << 'PY'
import struct, sys
w = sys.argv[1]
code = bytes([0xba, 0xf8, 0x03]) + b"".join(bytes([0xb0, c, 0xee]) for c in b"NEXA-VBR-OK\r\n") + bytes([0xfa, 0xf4, 0xeb, 0xfd])
with open(w + "/lost.img", "r+b") as f:
    f.seek(2048 * 512 + 0x54)
    f.write(code)
bcd = bytearray(b"regf" + bytes(4092))
for off in (2048 * 512, 206848 * 512, 1026048 * 512):
    d = bytearray(0x48)
    struct.pack_into("<I", d, 0, 6); struct.pack_into("<I", d, 8, 0x48); struct.pack_into("<Q", d, 16, off)
    struct.pack_into("<I", d, 36, 1); struct.pack_into("<I", d, 40, 0x1234ABCD)
    bcd += b"\xa0\xff\xff\xffent\0" + d
open(w + "/serve/BCD", "wb").write(bcd)
open(w + "/serve/bootmgr", "wb").write(b"BOOTMGR" * 1000)
PY
docker run --rm -v "$repo:/src" -v "$work:/s" -v /var/tmp/nexa-cargo:/root/.cargo/registry -w /src/tools nexa-rust:3.24 sh -c \
    't=$(rustc -vV | sed -n "s/^host: //p"); RUSTFLAGS="-C target-feature=+crt-static" cargo build --release -q -p nexa-bootfix --target $t
     cp target/$t/release/nexa-bootfix /s/serve/; chown -R '"$(id -u):$(id -g)"' /s/serve target'
cp "$repo/scripts/bootfix-test/scenario.sh" "$work/serve/t.sh"
( cd "$work/serve" && timeout 600 python3 -m http.server 8765 --bind 127.0.0.1 >/dev/null 2>&1 & )
NEXA_TEST_NVME="$work/win.img" NEXA_TEST_NVME2="$work/lost.img" NEXA_TEST_USB2="$work/usb.img" NEXA_RUN_WAIT=400 \
    python3 "$repo/scripts/test-live-boot.py" --no-checks --shot "$work/desktop.png" \
    --run 'for i in $(seq 1 15); do wget -qO /root/t.sh http://10.0.2.2:8765/t.sh && break; sleep 3; done; sh /root/t.sh 2>&1' \
    --fetch /root/shot.png "$work/screenshot.png" > "$work/run.txt" 2>&1 || true
grep -E 'RESULT|DONE|NOT FIXED|Screenshot|RESTORED|COPIED|SKIPPED|UNCHANGED|FILES_|PICKED|^at [0-9]+ MB' "$work/run.txt" || true
fail=0
grep -q 'RESULT: 3 fault' "$work/run.txt" || { echo "FAIL: the broken disk wasn't reported"; fail=1; }
grep -q 'Wrote boot entry .*Windows Boot Manager' "$work/run.txt" || { echo "FAIL: no boot entry written"; fail=1; }
grep -q BOOTMGR_RESTORED "$work/run.txt" || { echo "FAIL: boot manager not put back"; fail=1; }
grep -q FALLBACK_RESTORED "$work/run.txt" || { echo "FAIL: fallback not put back"; fail=1; }
grep -q 'Screenshot saved' "$work/run.txt" || { echo "FAIL: no screenshot"; fail=1; }
grep -q 'report-before.txt' "$work/run.txt" || { echo "FAIL: no report on the stick"; fail=1; }
grep -q 'Windows (C:) (users: Bless)' "$work/run.txt" || { echo "FAIL: the deleted Windows partition wasn't found"; fail=1; }
grep -q FILES_AS_USER "$work/run.txt" || { echo "FAIL: Files didn't open as the desktop user"; fail=1; }
grep -q FILES_DRIVE_READONLY "$work/run.txt" || { echo "FAIL: the Windows drive in Files wasn't read-only"; fail=1; }
grep -q PICKED_COPIED "$work/run.txt" || { echo "FAIL: a file picked in Files didn't reach the stick"; fail=1; }
grep -q 'COPIED: 1 files' "$work/run.txt" || { echo "FAIL: the picked file wasn't counted"; fail=1; }
grep -q NOTES_COPIED "$work/run.txt" || { echo "FAIL: Documents not copied"; fail=1; }
grep -q PHOTO_COPIED "$work/run.txt" || { echo "FAIL: Pictures not copied"; fail=1; }
grep -q APPDATA_SKIPPED "$work/run.txt" || { echo "FAIL: AppData was copied"; fail=1; }
grep -q LOST_DISK_UNCHANGED "$work/run.txt" || { echo "FAIL: the disk with the deleted partitions changed before the restore was asked for"; fail=1; }
grep -q 'Windows boot (Legacy BIOS)' "$work/run.txt" || { echo "FAIL: the Legacy BIOS boot partition wasn't recognised"; fail=1; }
grep -q MBR_SIG_OK "$work/run.txt" || { echo "FAIL: the MBR wasn't written with Windows' disk ID"; fail=1; }
grep -q KERNEL_SEES_PARTITIONS "$work/run.txt" || { echo "FAIL: the restored partitions aren't seen"; fail=1; }
grep -q C_MOUNTS "$work/run.txt" || { echo "FAIL: the restored C: doesn't open"; fail=1; }
grep -q 'starts in Legacy BIOS mode' "$work/run.txt" || { echo "FAIL: no Legacy BIOS steps after the restore"; fail=1; }
grep -q UNDO_OK "$work/run.txt" || { echo "FAIL: undo didn't give back the disk as it was"; fail=1; }
# The restored disk, started by a plain BIOS (SeaBIOS): MBR code -> active partition.
timeout 30 qemu-system-x86_64 -m 128 -drive file="$work/lost.img",format=raw,if=ide -display none -no-reboot \
    -serial file:"$work/legacy-serial.txt" >/dev/null 2>&1 || true
grep -q NEXA-VBR-OK "$work/legacy-serial.txt" && echo "LEGACY_BOOT_OK: the BIOS started the restored disk's active partition" \
    || { echo "FAIL: the restored disk didn't start in Legacy BIOS mode"; fail=1; }
[ $fail = 0 ] && echo "PASS: nexa-bootfix found, fixed and reported the broken Windows boot, put back a Legacy BIOS partition list, and rescued the files from deleted partitions (log: $work/run.txt)"
exit $fail
