#!/bin/bash

set -e

echo "Compiling nexa-fbdesktop..."
cd /home/nexa/NexaOS/gui
make nexa-fbdesktop

echo "Compiling standalone GUI apps..."
mkdir -p /home/nexa/NexaOS/gui/build_apps
cc -Wall -O2 -static -o build_apps/clock apps/clock.c -lm
cc -Wall -O2 -static -o build_apps/calc apps/calc.c
cc -Wall -O2 -static -o build_apps/files apps/files.c
cc -Wall -O2 -static -o build_apps/term apps/term.c
cc -Wall -O2 -static -o build_apps/about apps/about.c
cc -Wall -O2 -static -o build_apps/editor apps/editor.c
cc -Wall -O2 -static -o build_apps/todo apps/todo.c
cc -Wall -O2 -static -o build_apps/sysmon apps/sysmon.c
cc -Wall -O2 -static -o build_apps/image apps/image.c
cc -Wall -O2 -static -o build_apps/archive apps/archive.c
cc -Wall -O2 -static -o build_apps/props apps/props.c
cc -Wall -O2 -static -o build_apps/csv_view apps/csv_view.c
cc -Wall -O2 -static -o build_apps/md_view apps/md_view.c
cc -Wall -O2 -static -o build_apps/audio apps/audio.c

echo "Mounting disk..."
mkdir -p /home/nexa/NexaOS/mnt
mount -o loop /home/nexa/NexaOS/nexa.img /home/nexa/NexaOS/mnt

echo "Installing to /apps/desktop-gui/..."
mkdir -p /home/nexa/NexaOS/mnt/apps/desktop-gui
cp /home/nexa/NexaOS/gui/nexa-fbdesktop /home/nexa/NexaOS/mnt/apps/desktop-gui/fbdesktop
cp /home/nexa/NexaOS/disk/apps/desktop-gui/run /home/nexa/NexaOS/mnt/apps/desktop-gui/run 2>/dev/null || true
cp /home/nexa/NexaOS/gui/nexa_wrapper.sh /home/nexa/NexaOS/mnt/apps/desktop-gui/nexa_wrapper.sh
chmod +x /home/nexa/NexaOS/mnt/apps/desktop-gui/fbdesktop
chmod +x /home/nexa/NexaOS/mnt/apps/desktop-gui/run 2>/dev/null || true
chmod +x /home/nexa/NexaOS/mnt/apps/desktop-gui/nexa_wrapper.sh

echo "Installing standalone apps to /apps/gui/..."
mkdir -p /home/nexa/NexaOS/mnt/apps/gui
cp /home/nexa/NexaOS/gui/build_apps/* /home/nexa/NexaOS/mnt/apps/gui/
chmod +x /home/nexa/NexaOS/mnt/apps/gui/*

echo "Setting up associations and data..."
mkdir -p /home/nexa/NexaOS/mnt/etc
mkdir -p /home/nexa/NexaOS/mnt/data

cat > /home/nexa/NexaOS/mnt/etc/associations.conf << 'EOF'
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

# Create a test PPM image
echo -e "P6\n64 64\n255" > /home/nexa/NexaOS/mnt/data/test.ppm
python3 -c "
f = open('/home/nexa/NexaOS/mnt/data/test.ppm', 'ab')
for y in range(64):
    for x in range(64):
        f.write(bytes([x*4, y*4, 128]))
f.close()
"

# Create a test TAR archive
echo "Hello from the extracted text file!" > /home/nexa/NexaOS/hello.txt
tar -cf /home/nexa/NexaOS/mnt/data/test.tar -C /home/nexa/NexaOS hello.txt
rm /home/nexa/NexaOS/hello.txt

# Create a test CSV
cat > /home/nexa/NexaOS/mnt/data/test.csv << 'EOF'
NAME,AGE,CITY
ALICE,24,NEW YORK
BOB,30,LONDON
CHARLIE,28,PARIS
EOF

# Create a test MD
cat > /home/nexa/NexaOS/mnt/data/readme.md << 'EOF'
# NEXA OS 3.0
Welcome to the latest version of Nexa OS.
- Now with custom pixel icons
- Context Menus
- Desktop Shortcuts
- Archive extraction
# DOCUMENTATION
You can read this file directly in the Markdown viewer!
EOF

# Create a test WAV (a simple 440Hz sine wave)
python3 -c "
import math, struct
f = open('/home/nexa/NexaOS/mnt/data/test.wav', 'wb')
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
"

echo "Setting up Desktop Shortcuts..."
mkdir -p /home/nexa/NexaOS/mnt/Desktop

cat > /home/nexa/NexaOS/mnt/Desktop/Terminal.desktop << 'EOF'
Name=TERMINAL
Exec=/disk/apps/gui/term &
Icon=0
EOF

cat > /home/nexa/NexaOS/mnt/Desktop/Clock.desktop << 'EOF'
Name=CLOCK
Exec=/disk/apps/gui/clock &
Icon=1
EOF

cat > /home/nexa/NexaOS/mnt/Desktop/Calc.desktop << 'EOF'
Name=CALC
Exec=/disk/apps/gui/calc &
Icon=2
EOF

cat > /home/nexa/NexaOS/mnt/Desktop/Files.desktop << 'EOF'
Name=FILES
Exec=/disk/apps/gui/files &
Icon=3
EOF

cat > /home/nexa/NexaOS/mnt/Desktop/System.desktop << 'EOF'
Name=SYSTEM
Exec=/disk/apps/gui/sysmon &
Icon=4
EOF

cat > /home/nexa/NexaOS/mnt/Desktop/Editor.desktop << 'EOF'
Name=EDITOR
Exec=/disk/apps/gui/editor &
Icon=5
EOF

cat > /home/nexa/NexaOS/mnt/Desktop/Todo.desktop << 'EOF'
Name=TODO
Exec=/disk/apps/gui/todo &
Icon=6
EOF

cat > /home/nexa/NexaOS/mnt/Desktop/About.desktop << 'EOF'
Name=ABOUT
Exec=/disk/apps/gui/about &
Icon=7
EOF

cat > /home/nexa/NexaOS/mnt/Desktop/Audio.desktop << 'EOF'
Name=AUDIO
Exec=/disk/apps/gui/audio &
Icon=3
EOF

echo "Unmounting disk..."
umount /home/nexa/NexaOS/mnt

echo "Deployment complete."
