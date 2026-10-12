#!/bin/bash
set -e

umount /home/nexa/NexaOS/mnt 2>/dev/null || true
mount -o loop /home/nexa/NexaOS/nexa.img /home/nexa/NexaOS/mnt

# Remove duplicate MOTD call
sed -i '/cat \/etc\/motd/d' /home/nexa/NexaOS/mnt/etc/profile

# Add ping alias if not present
if ! grep -q "alias ping=" /home/nexa/NexaOS/mnt/etc/profile; then
    echo "alias ping='/bin/ping'" >> /home/nexa/NexaOS/mnt/etc/profile
fi

umount /home/nexa/NexaOS/mnt
echo "Profile fixed."
