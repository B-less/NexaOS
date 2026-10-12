#!/bin/bash
set -e

umount /home/nexa/NexaOS/mnt 2>/dev/null || true
mount -o loop /home/nexa/NexaOS/nexa.img /home/nexa/NexaOS/mnt

# Use ash directly as login shell — login prefixes it with '-' making it a
# login shell, so ash sources /etc/profile exactly once. No wrapper needed.
cat > /home/nexa/NexaOS/mnt/etc/passwd << 'EOF'
root:x:0:0:root:/root:/bin/sh
nexa:x:1000:1000:Nexa User:/home/nexa:/bin/ash
EOF

# Also drop the export -f line from profile (busybox ash doesn't support it)
sed -i '/^export -f/d' /home/nexa/NexaOS/mnt/etc/profile

# Verify
echo "=== /etc/passwd ==="
cat /home/nexa/NexaOS/mnt/etc/passwd
echo "=== Last 5 lines of profile ==="
tail -5 /home/nexa/NexaOS/mnt/etc/profile

umount /home/nexa/NexaOS/mnt
echo "Done."
