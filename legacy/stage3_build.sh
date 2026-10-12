#!/bin/bash
set -e

BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"
DB_SRC="/tmp/dropbear-2022.83"

umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT

echo "Installing SSH Server..."
mkdir -p $MNT/etc/dropbear
mkdir -p $MNT/usr/sbin
mkdir -p $MNT/usr/bin

# Install binaries
cp $DB_SRC/dropbear $MNT/usr/sbin/dropbear
cp $DB_SRC/dbclient $MNT/usr/bin/dbclient
cp $DB_SRC/dropbearkey $MNT/usr/bin/dropbearkey
chmod +x $MNT/usr/sbin/dropbear $MNT/usr/bin/dbclient $MNT/usr/bin/dropbearkey

# Generate keys inside the mount
echo "Generating dropbear keys..."
chroot $MNT /usr/bin/dropbearkey -t rsa -f /etc/dropbear/dropbear_rsa_host_key
chroot $MNT /usr/bin/dropbearkey -t ecdsa -f /etc/dropbear/dropbear_ecdsa_host_key

# Inject sshd into rcS
if ! grep -q "dropbear" $MNT/etc/init.d/rcS; then
    sed -i '/crond 2>\/dev\/null/a dropbear -R -B 2>/dev/null' $MNT/etc/init.d/rcS
fi

echo "Creating Global Config System..."
# Setup /etc/nexa.conf defaults
cat > $MNT/etc/nexa.conf << 'EOF'
# Nexa OS Global Configuration
system.hostname=nexa
network.dns1=8.8.8.8
network.dns2=8.8.4.4
ui.theme=dark
EOF

# Create the config command tool
cat > $MNT/bin/config << 'EOF'
#!/bin/sh
CONF="/etc/nexa.conf"
case "$1" in
    get)
        [ -z "$2" ] && echo "Usage: config get <key>" && exit 1
        grep "^$2=" "$CONF" | cut -d'=' -f2-
        ;;
    set)
        [ -z "$3" ] && echo "Usage: config set <key> <value>" && exit 1
        if grep -q "^$2=" "$CONF"; then
            sed -i "s/^$2=.*/$2=$3/" "$CONF"
        else
            echo "$2=$3" >> "$CONF"
        fi
        echo "Set $2 = $3"
        ;;
    list)
        cat "$CONF"
        ;;
    *)
        echo "Usage: config <get|set|list> [args]"
        ;;
esac
EOF
chmod +x $MNT/bin/config

echo "Creating fetch networking wrapper..."
cat > $MNT/bin/fetch << 'EOF'
#!/bin/sh
if [ -z "$1" ]; then
    echo "Usage: fetch <url> [output_file]"
    exit 1
fi
URL="$1"
OUT="${2:-$(basename "$URL")}"
echo "Downloading $URL to $OUT..."
if wget -O "$OUT" "$URL"; then
    echo "Download complete."
else
    echo "Error: Download failed."
    rm -f "$OUT"
fi
EOF
chmod +x $MNT/bin/fetch

echo "Done."
umount $MNT
