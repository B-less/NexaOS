#!/bin/bash
set -e

umount /home/nexa/NexaOS/mnt 2>/dev/null || true
mount -o loop /home/nexa/NexaOS/nexa.img /home/nexa/NexaOS/mnt

# Set nexa user's login shell to the nexa-admin CLI
cat > /home/nexa/NexaOS/mnt/etc/passwd << 'EOF'
root:x:0:0:root:/root:/bin/sh
nexa:x:1000:1000:Nexa User:/home/nexa:/sbin/nexa-admin
EOF

# Also make sure nexa-admin is executable and has correct shebang
head -1 /home/nexa/NexaOS/mnt/sbin/nexa-admin
chmod +x /home/nexa/NexaOS/mnt/sbin/nexa-admin

echo "=== /etc/passwd ==="
cat /home/nexa/NexaOS/mnt/etc/passwd

umount /home/nexa/NexaOS/mnt
echo "Done."
