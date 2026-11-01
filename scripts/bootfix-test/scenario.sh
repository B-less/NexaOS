# Runs inside the live ISO as root (scripts/test-bootfix.sh serves it).
for i in $(seq 1 20); do wget -qO /usr/local/bin/nexa-bootfix http://10.0.2.2:8765/nexa-bootfix && break; sleep 3; done
chmod 755 /usr/local/bin/nexa-bootfix
ls /sys/block
mkdir -p /mnt/e /mnt/w
mount -t vfat /dev/nvme0n1p1 /mnt/e
mount -t ntfs3 /dev/nvme0n1p3 /mnt/w
mkdir -p /mnt/w/Windows/System32 /mnt/w/Windows/Boot/EFI
cp /mnt/e/EFI/Microsoft/Boot/bootmgfw.efi /mnt/w/Windows/Boot/EFI/bootmgfw.efi
: > /mnt/w/Windows/System32/winload.efi
head -c 3000 /mnt/w/Windows/Boot/EFI/bootmgfw.efi > /mnt/e/EFI/Microsoft/Boot/bootmgfw.efi
mv /mnt/e/EFI/Boot/bootx64.efi /mnt/e/EFI/Boot/old.bin
umount /mnt/e /mnt/w
echo "===== CHECK BEFORE"
nexa-bootfix check
echo "===== DRIVES"
nexa-bootfix drives
sleep 15
echo "===== INTERACTIVE"
printf 'yes\n\n' | nexa-bootfix
echo "===== CHECK AFTER"
nexa-bootfix check | head -12
echo "===== ON THE USB DRIVE"
for d in /dev/sd[b-z]1; do m=$(awk -v d=$d '$1==d{print $2}' /proc/mounts); [ -n "$m" ] || { mkdir -p /mnt/u; mount $d /mnt/u && m=/mnt/u; }; ls $m/NexaOS-bootfix-*/ $m/NexaOS-bootfix-*/backup/boot-partition/EFI/Microsoft/Boot 2>/dev/null && cp $m/NexaOS-bootfix-*/screenshot.png /root/shot.png; done
mount -o ro -t vfat /dev/nvme0n1p1 /mnt/e; mount -o ro -t ntfs3 /dev/nvme0n1p3 /mnt/w
cmp /mnt/e/EFI/Microsoft/Boot/bootmgfw.efi /mnt/w/Windows/Boot/EFI/bootmgfw.efi && echo BOOTMGR_RESTORED
cmp /mnt/e/EFI/Boot/bootx64.efi /mnt/w/Windows/Boot/EFI/bootmgfw.efi && echo FALLBACK_RESTORED
