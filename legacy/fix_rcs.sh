#!/bin/bash
set -e

umount /home/nexa/NexaOS/mnt 2>/dev/null || true
mount -o loop /home/nexa/NexaOS/nexa.img /home/nexa/NexaOS/mnt

cat > /home/nexa/NexaOS/mnt/etc/init.d/rcS << 'RCEOF'
#!/bin/sh
mount -a 2>/dev/null
hostname nexa
mkdir -p /var/log /var/run /var/spool/cron/crontabs /tmp
chmod 1777 /tmp
echo "nameserver 8.8.8.8" > /etc/resolv.conf
echo "nameserver 8.8.4.4" >> /etc/resolv.conf
ifconfig lo 127.0.0.1 up
ifconfig eth0 up
sleep 1
udhcpc -i eth0 -q -t 5 -T 3 -n 2>/dev/null && echo "Network: DHCP OK" || {
    echo "Network: DHCP failed - using static"
    ifconfig eth0 10.0.2.15 netmask 255.255.255.0
    route add default gw 10.0.2.2
}
echo "0 2147483647" > /proc/sys/net/ipv4/ping_group_range
[ -f /lib/modules/nexa_proc.ko ] && insmod /lib/modules/nexa_proc.ko 2>/dev/null && echo "Module: /proc/nexa loaded" || true
syslogd -L -O /var/log/messages 2>/dev/null
klogd 2>/dev/null
crond 2>/dev/null
echo "Nexa OS ready."
RCEOF

chmod +x /home/nexa/NexaOS/mnt/etc/init.d/rcS
echo "=== rcS ==="
cat /home/nexa/NexaOS/mnt/etc/init.d/rcS
umount /home/nexa/NexaOS/mnt
echo "Done."
