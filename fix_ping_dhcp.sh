#!/bin/bash
set -e

umount /home/nexa/NexaOS/mnt 2>/dev/null || true
mount -o loop /home/nexa/NexaOS/nexa.img /home/nexa/NexaOS/mnt

echo "Fixing ping SUID..."
chmod u+s /home/nexa/NexaOS/mnt/bin/ping
ls -la /home/nexa/NexaOS/mnt/bin/ping

echo "Fixing DHCP in rcS..."
cat > /home/nexa/NexaOS/mnt/etc/init.d/rcS << 'EOF'
#!/bin/sh

# Filesystem and basics
mount -a 2>/dev/null
hostname nexa
mkdir -p /var/log /var/run /var/spool/cron/crontabs /tmp
chmod 1777 /tmp

# DNS
echo "nameserver 8.8.8.8" > /etc/resolv.conf
echo "nameserver 8.8.4.4" >> /etc/resolv.conf

# Loopback
ifconfig lo 127.0.0.1 up

# Bring up eth0 and wait for DHCP synchronously
ifconfig eth0 up
sleep 1
udhcpc -i eth0 -q -t 5 -T 3 -n 2>/dev/null && echo "Network: DHCP OK" || echo "Network: DHCP failed"

# Load nexa kernel module
[ -f /lib/modules/nexa_proc.ko ] && insmod /lib/modules/nexa_proc.ko 2>/dev/null && echo "Module: /proc/nexa loaded" || true

# Start system daemons
syslogd -L -O /var/log/messages 2>/dev/null
klogd 2>/dev/null
crond 2>/dev/null

echo "Nexa OS ready."
EOF
chmod +x /home/nexa/NexaOS/mnt/etc/init.d/rcS

echo "Unmounting..."
umount /home/nexa/NexaOS/mnt
echo "Done."
