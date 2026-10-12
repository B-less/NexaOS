#!/bin/bash

echo "================================="
echo "  NEXA OS - DISK SETUP SCRIPT"
echo "================================="
echo

# Step 1: Create 1 GB virtual disk
echo "[Step 1] Creating 1 GB virtual disk..."
cd ~/NexaOS
qemu-img create -f raw nexa.img 1G
echo "✓ Disk created"
echo

# Verify disk
echo "Verifying disk:"
ls -lh nexa.img
echo

# Step 2: Format the disk with ext4
echo "[Step 2] Formatting disk with ext4..."
sudo apt install -y e2fsprogs > /dev/null 2>&1
mkfs.ext4 nexa.img
echo "✓ Filesystem created"
echo

# Step 3: Mount the disk
echo "[Step 3] Mounting disk..."
mkdir -p ~/NexaOS/mnt
sudo mount -o loop ~/NexaOS/nexa.img ~/NexaOS/mnt
echo "✓ Disk mounted to ~/NexaOS/mnt"
echo

# Verify mount
echo "Verifying mount:"
df -h | grep nexa
echo

# Step 4: Create Nexa directories
echo "[Step 4] Creating directories on disk..."
sudo mkdir -p ~/NexaOS/mnt/apps
sudo mkdir -p ~/NexaOS/mnt/packages
sudo mkdir -p ~/NexaOS/mnt/home
echo "✓ Directories created"
echo

# Create package database
echo "[Step 4b] Creating package database..."
sudo touch ~/NexaOS/mnt/packages/installed.db
echo "about" | sudo tee -a ~/NexaOS/mnt/packages/installed.db > /dev/null
echo "system" | sudo tee -a ~/NexaOS/mnt/packages/installed.db > /dev/null
echo "calculator" | sudo tee -a ~/NexaOS/mnt/packages/installed.db > /dev/null
echo "✓ Package database created"
echo

# Step 5: Copy apps onto the disk
echo "[Step 5] Copying apps to disk..."
sudo cp -r ~/NexaOS/rootfs/apps/* ~/NexaOS/mnt/apps/
echo "✓ Apps copied"
echo

# Verify apps
echo "Verifying apps on disk:"
find ~/NexaOS/mnt/apps -type f -name "run" | head -10
echo

# Show disk contents
echo "Disk contents:"
ls -lh ~/NexaOS/mnt/
echo

# Step 6: Unmount
echo "[Step 6] Unmounting disk..."
sudo umount ~/NexaOS/mnt
echo "✓ Disk unmounted"
echo

echo "================================="
echo "  SETUP COMPLETE"
echo "================================="
echo
echo "To boot QEMU with the disk, run:"
echo
echo "qemu-system-x86_64 \\"
echo "  -kernel ~/NexaOS/kernel/linux-6.18.33/arch/x86/boot/bzImage \\"
echo "  -initrd ~/NexaOS/initramfs.cpio.gz \\"
echo "  -append \"console=ttyS0 rdinit=/init\" \\"
echo "  -drive file=~/NexaOS/nexa.img,format=raw \\"
echo "  -nographic"
echo
