#!/bin/bash
set -e
BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"

umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT

# Copy the TUI source code
cp /mnt/c/Users/BBF/nexa_tui.c $MNT/home/nexa/nexa_tui.c
chown 1000:1000 $MNT/home/nexa/nexa_tui.c

# Add the dashboard alias to /etc/profile
cat << 'EOF' >> $MNT/etc/profile

# Nexa OS Control Center
dash() {
    cd /home/nexa
    if [ ! -x ./nexa_tui ] || [ nexa_tui.c -nt nexa_tui ]; then
        echo "Compiling Control Center..."
        tcc nexa_tui.c -lncurses -o nexa_tui
    fi
    ./nexa_tui
}
EOF

umount $MNT
echo "TUI injected."
