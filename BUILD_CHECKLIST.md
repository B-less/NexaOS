# NexaOS Build Verification Checklist

## Phase 1: Kernel & Core Build (CRITICAL)

### Kernel 6.18.33
- [ ] Kernel source exists in `kernel/linux-6.18.33/`
- [ ] `.config` file exists and includes DRM support
- [ ] Required options configured:
  - [ ] `CONFIG_DRM=y` (Direct Rendering Manager)
  - [ ] `CONFIG_DRM_VIRTIO_GPU=y` (QEMU GPU)
  - [ ] `CONFIG_VIRTIO=y` (Virtio devices)
  - [ ] `CONFIG_FB=y` (Framebuffer)
  - [ ] `CONFIG_INPUT_EVDEV=y` (Input devices)
  - [ ] `CONFIG_TMPFS=y` (tmpfs filesystem)
  - [ ] `CONFIG_EXT4_FS=y` (ext4 filesystem)

**Verify:**
```bash
grep CONFIG_DRM kernel/linux-6.18.33/.config
# Should output: CONFIG_DRM=y
```

### Core Build Dependencies
- [ ] `gcc` installed
- [ ] `make` installed
- [ ] `qemu-img` installed
- [ ] `curl` installed for Alpine download

**Verify:**
```bash
which gcc make qemu-img curl
```

### Makefile Syntax
- [ ] No syntax errors in Makefile
- [ ] build.conf included and readable

**Verify:**
```bash
make check-env
# Should show: ✓ Environment verified
```

---

## Phase 2: Rootfs Bootstrap (CRITICAL)

### Alpine Linux Download
- [ ] Alpine minirootfs downloads successfully
- [ ] SHA256 checksum verified (optional but recommended)
- [ ] Cached in `.build_cache/`

**Verify:**
```bash
make alpine-download
ls -lh .build_cache/alpine-minirootfs-*.tar.gz
```

### Rootfs Extraction
- [ ] Alpine extracted to `rootfs/` directory
- [ ] All Alpine directories present: `bin`, `etc`, `lib`, `usr`, `var`, `root`, `home`
- [ ] APK database initialized: `lib/apk/db/installed`

**Verify:**
```bash
make rootfs
ls -la rootfs/{bin,etc,lib,usr,var,root,home}
file rootfs/lib/apk/db/installed
```

### Configuration Files
- [ ] `/etc/hostname` set to "nexa"
- [ ] `/etc/resolv.conf` configured
- [ ] `/etc/fstab` created
- [ ] `/etc/passwd` and `/etc/shadow` created
- [ ] User/group databases correct

**Verify:**
```bash
cat rootfs/etc/hostname         # Should output: nexa
cat rootfs/etc/resolv.conf      # Should have nameservers
cat rootfs/etc/passwd           # Should have root:user entries
```

---

## Phase 3: Initramfs & Boot (CRITICAL)

### Initramfs Generation
- [ ] `dist/initramfs.cpio.gz` created
- [ ] Size reasonable (>1MB, <50MB)
- [ ] Can be decompressed successfully

**Verify:**
```bash
make initramfs
ls -lh dist/initramfs.cpio.gz
gunzip -t dist/initramfs.cpio.gz
```

### Kernel Build
- [ ] `dist/bzImage` exists
- [ ] Size reasonable (>5MB, <30MB)
- [ ] Can be read by QEMU

**Verify:**
```bash
make kernel
ls -lh dist/bzImage
file dist/bzImage     # Should show: Linux kernel x86 boot executable
```

---

## Phase 4: Disk Image & APK (CRITICAL)

### Disk Image Creation
- [ ] `dist/nexa.img` created (1GB ext4)
- [ ] Image is valid ext4
- [ ] Can be mounted/unmounted

**Verify:**
```bash
make disk
ls -lh dist/nexa.img
file dist/nexa.img     # Should show: Linux rev 1.0 ext4 filesystem
```

### APK Package Installation
**NOTE**: APK installation inside chroot requires:
- [ ] `sudo` access (for mount/umount/chroot)
- [ ] Network access (to download packages)
- [ ] qemu-user-static or native x86_64 (for chroot)

**Verify in disk target:**
- [ ] APK updates successfully: `apk update`
- [ ] Core packages install: `apk add alpine-base`
- [ ] Wayland packages install: `apk add sway waybar`
- [ ] No installation errors (check chroot output)

**Check:**
```bash
sudo chroot rootfs /sbin/apk info | head -10
# Should list installed packages
```

---

## Phase 5: Sway Configuration (IMPORTANT)

### Configuration Files
- [ ] `rootfs/etc/sway/config` created and valid
- [ ] `rootfs/etc/xdg/waybar/config` created and valid
- [ ] User config directories: `~/.config/sway`, `~/.config/waybar`

**Verify:**
```bash
test -f rootfs/etc/sway/config && echo "✓ Sway config"
test -f rootfs/etc/xdg/waybar/config && echo "✓ Waybar config"
```

### OpenRC Services
- [ ] `/etc/init.d/sway` service created
- [ ] Service is executable
- [ ] Init system will start it on boot

**Verify:**
```bash
test -x rootfs/etc/init.d/sway && echo "✓ Sway service"
```

---

## Phase 6: First Boot Testing

### Serial Boot Test
```bash
make boot
# Expected output:
# - Kernel boot messages
# - "Alpine Linux" init system starts
# - Login prompt appears (ttyS0)
# - Can type commands (ash shell)
```

**Verify:**
- [ ] Kernel boots
- [ ] Filesystem mounted as root
- [ ] Login shell responds
- [ ] `ls`, `ps`, `cat` work

### GUI Boot Test
```bash
make boot-gui
# Expected output:
# - Kernel boots
# - DRM/GPU initialized
# - Wayland/Sway starts
# - Desktop environment visible
```

**Verify:**
- [ ] QEMU window opens
- [ ] Desktop appears (Sway wallpaper)
- [ ] Waybar visible (top/bottom of screen)
- [ ] Can move mouse
- [ ] Terminal opens: `mod+return` (Super+Enter)

---

## Phase 7: ISO Creation (ADVANCED)

### Prerequisites
- [ ] `xorriso` installed
- [ ] GRUB tools installed (optional, for UEFI)
- [ ] ISOLINUX installed (for BIOS fallback)

**Verify:**
```bash
which xorriso
ls -la /usr/lib/ISOLINUX/isohdpfx.bin
```

### ISO Generation
```bash
make iso
# Creates: dist/NexaOS-3.1.0.iso
```

**Verify:**
- [ ] ISO file created: `dist/NexaOS-3.1.0.iso`
- [ ] Size reasonable (>500MB, <2GB)
- [ ] Can be mounted: `sudo mount -o loop dist/NexaOS-3.1.0.iso /mnt`

---

## Troubleshooting Matrix

| Issue | Cause | Solution |
|-------|-------|----------|
| Alpine download fails | No internet | Check network connectivity |
| APK installation fails | Missing sudo | Run with `sudo` or fix chroot permissions |
| Kernel DRM missing | `.config` incomplete | Add `CONFIG_DRM=y` and rebuild |
| Sway won't start | Initramfs broken | Verify initramfs: `gunzip -t dist/initramfs.cpio.gz` |
| Boot hangs | Missing rootfs packages | Run `make distclean && make disk` |
| GPU not detected | Virtio GPU missing | Add `-device virtio-gpu-pci` in boot-gui target |

---

## Quick Start: First Build

### Step 1: Verify Environment
```bash
make check-env
```

### Step 2: First Build (download + compile)
```bash
make distclean  # Clean slate
make all        # Full build
```

**Estimated time**: 15-30 minutes (depending on network/CPU)

### Step 3: Test Serial Boot
```bash
make boot
# Press Ctrl+A then X to exit
```

### Step 4: Test GUI Boot
```bash
make boot-gui
# Close QEMU window to exit
```

### Step 5: Create ISO (optional)
```bash
make iso
```

---

## Known Limitations (v3.1.0)

1. **APK installation inside chroot** may require manual tweaking for non-standard environments
2. **GPU acceleration** in QEMU depends on host support (works on Linux with KVM)
3. **Wayland/Sway** keyboard layout may default to US (configurable in `etc/sway/config`)
4. **Network** inside QEMU may need manual configuration (DHCP not automatic)

---

## Success Criteria

✅ **Build is successful when:**
1. `make all` completes without errors
2. `dist/nexa.img` is ~1GB
3. `dist/initramfs.cpio.gz` is valid
4. `dist/bzImage` is readable
5. `make boot` shows Alpine login shell
6. `make boot-gui` shows Sway desktop
7. Mouse/keyboard work in GUI
8. Terminal opens in Sway

---

## Next Actions

1. **Run**: `make setup && make check-env`
2. **Build**: `make all` (watch for errors)
3. **Test**: `make boot` (verify serial boot)
4. **Test**: `make boot-gui` (verify GPU/Wayland)
5. **Report**: Document any build errors/fixes
6. **Plan**: Next phase (applications, additional packages)
