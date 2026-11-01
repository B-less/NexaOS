# Runs inside the live ISO as root (scripts/test-bootfix.sh serves it).
for i in $(seq 1 20); do wget -qO /usr/local/bin/nexa-bootfix http://10.0.2.2:8765/nexa-bootfix && break; sleep 3; done
chmod 755 /usr/local/bin/nexa-bootfix
ls /sys/block
# The two NVMe disks, by serial: W has a broken Windows boot, L deleted partitions.
for d in /sys/block/nvme*n1; do
    case "$(cat $d/device/serial)" in *NEXATEST*) W=${d##*/};; *NEXALOST*) L=${d##*/};; esac
done
echo "broken boot: $W, deleted partitions: $L"
mkdir -p /mnt/e /mnt/w /mnt/l
mount -t vfat /dev/${W}p1 /mnt/e
mount -t ntfs3 /dev/${W}p3 /mnt/w
mkdir -p /mnt/w/Windows/System32 /mnt/w/Windows/Boot/EFI
cp /mnt/e/EFI/Microsoft/Boot/bootmgfw.efi /mnt/w/Windows/Boot/EFI/bootmgfw.efi
: > /mnt/w/Windows/System32/winload.efi
head -c 3000 /mnt/w/Windows/Boot/EFI/bootmgfw.efi > /mnt/e/EFI/Microsoft/Boot/bootmgfw.efi
mv /mnt/e/EFI/Boot/bootx64.efi /mnt/e/EFI/Boot/old.bin
umount /mnt/e /mnt/w
# Windows (Legacy BIOS) with a user's files; its partition list is already
# empty, so the partitions are reached through loop devices at their starts.
lo() { D=$(losetup -f); losetup -o $(( $1 * 512 )) $D /dev/$L; echo $D; }
S=$(lo 2048); mount -t ntfs3 $S /mnt/l
mkdir -p /mnt/l/Boot
wget -qO /mnt/l/bootmgr http://10.0.2.2:8765/bootmgr; wget -qO /mnt/l/Boot/BCD http://10.0.2.2:8765/BCD
umount /mnt/l; losetup -d $S
C=$(lo 206848); mount -t ntfs3 $C /mnt/l
mkdir -p /mnt/l/Windows/System32 /mnt/l/Users/Bless/Documents /mnt/l/Users/Bless/Pictures /mnt/l/Users/Bless/AppData/Local /mnt/l/Users/Default
echo NEXA-LOST-NOTES > /mnt/l/Users/Bless/Documents/notes.txt
head -c 300000 /dev/urandom > /root/photo.jpg
cp /root/photo.jpg /mnt/l/Users/Bless/Pictures/photo.jpg
head -c 5000 /dev/urandom > /mnt/l/Users/Bless/AppData/Local/cache.bin
umount /mnt/l; losetup -d $C
R=$(lo 1026048); mount -t ntfs3 $R /mnt/l; mkdir -p /mnt/l/Recovery/WindowsRE; umount /mnt/l; losetup -d $R
sync
ls /sys/block/$L
before=$(sha256sum /dev/$L | cut -c1-64)
echo "===== CHECK BEFORE"
nexa-bootfix check
echo "===== DRIVES"
nexa-bootfix drives
sleep 15
# Plays the person at the desktop while Files is open: checks Files runs as
# them on a read-only Windows drive, copies one photo to the stick, takes a
# screenshot, closes Files.
picker() {
    for i in $(seq 1 60); do pgrep -f nexa_files >/dev/null && break; sleep 1; done
    sleep 6
    ps -o user,args | grep '[n]exa_files' | grep -q '^user ' && echo FILES_AS_USER
    M=$(awk '$2 ~ /Windows-C/ {print $2}' /proc/mounts | head -1)
    echo "windows drive: $M"
    su user -s /bin/sh -c "touch '$M/x' 2>/dev/null" || echo FILES_DRIVE_READONLY
    U=$(awk '$1 ~ /^\/dev\/sd[b-z]1$/ {print $2}' /proc/mounts | head -1)
    su user -s /bin/sh -c "cp '$M/Users/Bless/Pictures/photo.jpg' '$U/'" && echo PICKED_AS_USER
    su user -s /bin/sh -c 'XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=$(ls /run/user/1000 | grep "^wayland-[0-9]*$" | head -1) grim /tmp/files.png' && cp /tmp/files.png /root/shot.png
    pkill -f nexa_files
}
echo "===== INTERACTIVE (pick in Files)"
picker &
( echo 1; sleep 50; echo; echo no; echo ) | nexa-bootfix
wait
[ "$(sha256sum /dev/$L | cut -c1-64)" = "$before" ] && echo LOST_DISK_UNCHANGED
echo "===== INTERACTIVE (whole folders)"
printf '2\nall\nyes\n\n' | nexa-bootfix
echo "===== RESTORED PARTITION LIST"
[ "$(dd if=/dev/$L bs=1 skip=440 count=4 2>/dev/null | od -An -tx1 | tr -d ' ')" = "cdab3412" ] \
    && [ "$(dd if=/dev/$L bs=1 skip=510 count=2 2>/dev/null | od -An -tx1 | tr -d ' ')" = "55aa" ] && echo MBR_SIG_OK
ls /sys/block/$L
[ -e /sys/block/$L/${L}p1 ] && [ -e /sys/block/$L/${L}p2 ] && [ -e /sys/block/$L/${L}p3 ] && echo KERNEL_SEES_PARTITIONS
mount -o ro -t ntfs3 /dev/${L}p2 /mnt/l && [ -d /mnt/l/Windows/System32 ] && echo C_MOUNTS; umount /mnt/l
nexa-bootfix check | grep -A2 'Legacy BIOS'
echo "===== UNDO, THEN AGAIN"
B=$(ls -d /media/sd[b-z]1/NexaOS-bootfix-*/backup/disk.txt | head -1)
nexa-bootfix undo-partitions "${B%/backup/disk.txt}" --yes
[ "$(sha256sum /dev/$L | cut -c1-64)" = "$before" ] && echo UNDO_OK
nexa-bootfix fix --yes --to /root/redo | grep -E 'DONE|NOT'
sync
echo "===== CHECK AFTER"
nexa-bootfix check | head -12
echo "===== ON THE USB DRIVE"
for d in /dev/sd[b-z]1; do m=$(awk -v d=$d '$1==d{print $2}' /proc/mounts); [ -n "$m" ] || { mkdir -p /mnt/u; mount $d /mnt/u && m=/mnt/u; }
    ls $m/NexaOS-bootfix-*/ $m/NexaOS-bootfix-*/backup/boot-partition/EFI/Microsoft/Boot 2>/dev/null
    cat $m/NexaOS-bootfix-*/files-copied.txt 2>/dev/null
    cmp $m/photo.jpg /root/photo.jpg && echo PICKED_COPIED
    F=$(ls -d $m/NexaOS-bootfix-*/Files 2>/dev/null | head -1)
    if [ -n "$F" ]; then
        find "$F"
        grep -q NEXA-LOST-NOTES "$F/Bless/Documents/notes.txt" && echo NOTES_COPIED
        cmp "$F/Bless/Pictures/photo.jpg" /root/photo.jpg && echo PHOTO_COPIED
        [ ! -e "$F/Bless/AppData" ] && echo APPDATA_SKIPPED
    fi
done
mount -o ro -t vfat /dev/${W}p1 /mnt/e; mount -o ro -t ntfs3 /dev/${W}p3 /mnt/w
cmp /mnt/e/EFI/Microsoft/Boot/bootmgfw.efi /mnt/w/Windows/Boot/EFI/bootmgfw.efi && echo BOOTMGR_RESTORED
cmp /mnt/e/EFI/Boot/bootx64.efi /mnt/w/Windows/Boot/EFI/bootmgfw.efi && echo FALLBACK_RESTORED
