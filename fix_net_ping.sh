#!/bin/bash
set -e

echo "Compiling custom ping..."
cc -Wall -O2 -static -o /home/nexa/NexaOS/ping_bin /home/nexa/NexaOS/ping.c
echo "OK"

umount /home/nexa/NexaOS/mnt 2>/dev/null || true
mount -o loop /home/nexa/NexaOS/nexa.img /home/nexa/NexaOS/mnt

echo "Installing ping..."
rm -f /home/nexa/NexaOS/mnt/bin/ping
cp /home/nexa/NexaOS/ping_bin /home/nexa/NexaOS/mnt/bin/ping
chmod +x /home/nexa/NexaOS/mnt/bin/ping

echo "Fixing network configuration in rcS..."
# Use guaranteed QEMU static config + proper udhcpc script
cat > /home/nexa/NexaOS/mnt/etc/init.d/rcS << 'RCEOF'
#!/bin/sh
mount -a 2>/dev/null
hostname nexa
mkdir -p /var/log /var/run /var/spool/cron/crontabs /tmp
chmod 1777 /tmp

# Network setup for QEMU user networking
# QEMU always provides: IP=10.0.2.15, GW=10.0.2.2, DNS=10.0.2.3
ifconfig lo 127.0.0.1 up
ifconfig eth0 10.0.2.15 netmask 255.255.255.0 up
route add default gw 10.0.2.2 2>/dev/null || true
echo "nameserver 10.0.2.3" > /etc/resolv.conf
echo "nameserver 8.8.8.8" >> /etc/resolv.conf
echo "Network: 10.0.2.15 gw 10.0.2.2 OK"

# Load nexa kernel module
[ -f /lib/modules/nexa_proc.ko ] && insmod /lib/modules/nexa_proc.ko 2>/dev/null && echo "Module: /proc/nexa loaded" || true

# Start system daemons
syslogd -L -O /var/log/messages 2>/dev/null
klogd 2>/dev/null
crond 2>/dev/null

echo "Nexa OS ready."
RCEOF
chmod +x /home/nexa/NexaOS/mnt/etc/init.d/rcS

echo "Verifying..."
cat /home/nexa/NexaOS/mnt/etc/init.d/rcS

umount /home/nexa/NexaOS/mnt
echo "Done!"
