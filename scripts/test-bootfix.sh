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
#   nexa-bootfix must find the partitions, copy the folders to the stick
#   (not AppData) and leave that disk byte-for-byte unchanged.
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
sgdisk -o -n1:2048:+100M -t1:ef00 -n2:0:+16M -t2:0c01 -n3:0:0 -t3:0700 lost.img >/dev/null
s1=$(sgdisk -i1 lost.img | sed -n "s/First sector: \([0-9]*\).*/\1/p"); e1=$(sgdisk -i1 lost.img | sed -n "s/Last sector: \([0-9]*\).*/\1/p")
s3=$(sgdisk -i3 lost.img | sed -n "s/First sector: \([0-9]*\).*/\1/p"); e3=$(sgdisk -i3 lost.img | sed -n "s/Last sector: \([0-9]*\).*/\1/p")
mkfs.vfat -F32 -n SYSTEM --offset "$s1" lost.img $(( (e1-s1+1)/2 )) >/dev/null 2>&1
mmd -i "lost.img@@$((s1*512))" ::/EFI ::/EFI/Microsoft ::/EFI/Microsoft/Boot
truncate -s 0 /tmp/ntfs2.img; truncate -s $(( (e3-s3+1)*512 )) /tmp/ntfs2.img
mkntfs -q -F -f -L Windows /tmp/ntfs2.img >/dev/null 2>&1
dd if=/tmp/ntfs2.img of=lost.img bs=512 seek="$s3" conv=notrunc status=none
truncate -s 64M usb.img
echo type=c | sfdisk -q usb.img
mkfs.vfat -F32 -n REPORTS --offset 2048 usb.img $(( (131072-2048)/2 )) >/dev/null 2>&1
chmod 666 win.img usb.img lost.img'
docker run --rm -v "$repo:/src" -v "$work:/s" -v /var/tmp/nexa-cargo:/root/.cargo/registry -w /src/tools nexa-rust:3.24 sh -c \
    't=$(rustc -vV | sed -n "s/^host: //p"); RUSTFLAGS="-C target-feature=+crt-static" cargo build --release -q -p nexa-bootfix --target $t
     cp target/$t/release/nexa-bootfix /s/serve/; chown -R '"$(id -u):$(id -g)"' /s/serve target'
cp "$repo/scripts/bootfix-test/scenario.sh" "$work/serve/t.sh"
( cd "$work/serve" && timeout 600 python3 -m http.server 8765 --bind 127.0.0.1 >/dev/null 2>&1 & )
NEXA_TEST_NVME="$work/win.img" NEXA_TEST_NVME2="$work/lost.img" NEXA_TEST_USB2="$work/usb.img" NEXA_RUN_WAIT=400 \
    python3 "$repo/scripts/test-live-boot.py" --no-checks --shot "$work/desktop.png" \
    --run 'for i in $(seq 1 15); do wget -qO /root/t.sh http://10.0.2.2:8765/t.sh && break; sleep 3; done; sh /root/t.sh 2>&1' \
    --fetch /root/shot.png "$work/screenshot.png" > "$work/run.txt" 2>&1 || true
grep -E 'RESULT|DONE|NOT FIXED|Screenshot|RESTORED|COPIED|SKIPPED|UNCHANGED|^at [0-9]+ MB' "$work/run.txt" || true
fail=0
grep -q 'RESULT: 3 fault' "$work/run.txt" || { echo "FAIL: the broken disk wasn't reported"; fail=1; }
grep -q 'Wrote boot entry .*Windows Boot Manager' "$work/run.txt" || { echo "FAIL: no boot entry written"; fail=1; }
grep -q BOOTMGR_RESTORED "$work/run.txt" || { echo "FAIL: boot manager not put back"; fail=1; }
grep -q FALLBACK_RESTORED "$work/run.txt" || { echo "FAIL: fallback not put back"; fail=1; }
grep -q 'Screenshot saved' "$work/run.txt" || { echo "FAIL: no screenshot"; fail=1; }
grep -q 'report-before.txt' "$work/run.txt" || { echo "FAIL: no report on the stick"; fail=1; }
grep -q 'Windows (C:) (users: Bless)' "$work/run.txt" || { echo "FAIL: the deleted Windows partition wasn't found"; fail=1; }
grep -q NOTES_COPIED "$work/run.txt" || { echo "FAIL: Documents not copied"; fail=1; }
grep -q PHOTO_COPIED "$work/run.txt" || { echo "FAIL: Pictures not copied"; fail=1; }
grep -q APPDATA_SKIPPED "$work/run.txt" || { echo "FAIL: AppData was copied"; fail=1; }
grep -q LOST_DISK_UNCHANGED "$work/run.txt" || { echo "FAIL: the disk with the deleted partitions changed"; fail=1; }
[ $fail = 0 ] && echo "PASS: nexa-bootfix found, fixed and reported the broken Windows boot, and rescued the files from deleted partitions (log: $work/run.txt)"
exit $fail
