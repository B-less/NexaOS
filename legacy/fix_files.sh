#!/bin/bash

# Mount the disk
mkdir -p /mnt/nexa_disk
mount -o loop /home/nexa/NexaOS/nexa.img /mnt/nexa_disk

# 1. Re-create the .wav file directly
cat << 'EOF' > /home/nexa/NexaOS/make_wav.py
import math, struct
f = open('/mnt/nexa_disk/data/test.wav', 'wb')
sample_rate = 8000
duration = 3.0
n_samples = int(sample_rate * duration)
f.write(b'RIFF' + struct.pack('<I', 36 + n_samples) + b'WAVEfmt ')
f.write(struct.pack('<IHHIIHH', 16, 1, 1, sample_rate, sample_rate, 1, 8))
f.write(b'data' + struct.pack('<I', n_samples))
for i in range(n_samples):
    val = int(127.0 * math.sin(2.0 * math.pi * 440.0 * (i / sample_rate))) + 128
    f.write(bytes([val]))
f.close()
EOF
python3 /home/nexa/NexaOS/make_wav.py

# 2. Re-create the .tar file using strictly POSIX ustar format
echo "Hello from the extracted text file!" > /home/nexa/NexaOS/hello.txt
tar --format=ustar -cf /mnt/nexa_disk/data/test.tar -C /home/nexa/NexaOS hello.txt

# Also fix associations.conf in case it got messed up
cat > /mnt/nexa_disk/etc/associations.conf << 'EOF'
.txt=/disk/apps/gui/editor
.c=/disk/apps/gui/editor
.h=/disk/apps/gui/editor
.sh=/disk/apps/gui/editor
.ppm=/disk/apps/gui/image
.bmp=/disk/apps/gui/image
.tar=/disk/apps/gui/archive
.csv=/disk/apps/gui/csv_view
.md=/disk/apps/gui/md_view
.wav=/disk/apps/gui/audio
EOF

# Unmount
umount /mnt/nexa_disk
echo "Files fixed!"
