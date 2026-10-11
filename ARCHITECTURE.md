# NexaOS - Production Architecture

## Overview

NexaOS is a **modern Linux-based desktop operating system** built from scratch with a real, production-grade technology stack.

- **Base**: Alpine Linux 3.19.1 (minimal, musl libc)
- **Kernel**: Linux 6.18.33 (optimized for desktop with DRM/GPU support)
- **Display Server**: Wayland 1.22.0
- **Compositor**: Sway 1.8.1 (tiling window manager)
- **Taskbar**: Waybar 1.9.1
- **Launcher**: Wofi
- **Terminal**: Foot
- **Package Manager**: APK (Alpine Package Keeper)

## Why This Stack?

| Component | Why |
|-----------|-----|
| **Alpine Linux** | Minimal (100MB base), security-first, production-used in containers |
| **musl libc** | Smaller than glibc, statically linkable, reliable |
| **Wayland** | Modern display protocol, replacing X11 (deprecated) |
| **Sway** | Lightweight compositor, great for QEMU/minimal hardware |
| **APK** | Fast, deterministic, excellent dependency resolution |

## Architecture Diagram

```
┌─────────────────────────────────────────────────────┐
│           NexaOS Desktop Environment                 │
├─────────────────────────────────────────────────────┤
│  Applications: Foot, Thunar, GIMP, Firefox, etc.    │
├─────────────────────────────────────────────────────┤
│         Wayland Protocols & XDG Base Dirs            │
├─────────────────────────────────────────────────────┤
│  Sway Compositor (tiling, Wayland implementation)   │
├─────────────────────────────────────────────────────┤
│        Wayland Server (libwayland-server)            │
├─────────────────────────────────────────────────────┤
│  wlroots (Wayland rendering, GPU access)            │
├─────────────────────────────────────────────────────┤
│  Mesa/DRM (GPU drivers, framebuffer access)         │
├─────────────────────────────────────────────────────┤
│  Linux Kernel 6.18.33 (DRM, virtio-gpu, KMS)        │
├─────────────────────────────────────────────────────┤
│  Alpine Linux (OpenRC, APK package manager)         │
└─────────────────────────────────────────────────────┘
```

## Build System

The Makefile orchestrates a **5-stage build**:

### Stage 1: Environment Setup (`make setup`)
- Create build directories
- Verify dependencies
- Initialize logs

### Stage 2: Kernel Build (`make kernel`)
- Compile Linux 6.18.33 for x86_64
- Kernel modules support
- DRM/GPU drivers enabled

### Stage 3: Root Filesystem (`make rootfs`)
- Download Alpine minirootfs
- Configure APK repositories
- Setup Wayland environment variables
- Create user/group databases

### Stage 4: Disk Image Population (`make disk`)
- Mount ext4 disk image
- Copy Alpine rootfs
- Use APK to install packages into rootfs
  - Core: alpine-base, busybox, util-linux
  - Wayland: wayland, libxkbcommon, mesa
  - Sway: sway, waybar, wofi, swaylock
  - Apps: foot, xdg-desktop-portal-wlr, grim, slurp
- Configure Sway and Waybar
- Create OpenRC services

### Stage 5: Bootable ISO (`make iso`)
- Create ISO with GRUB 2 (UEFI) + ISOLINUX (BIOS)
- Hybrid image supports both physical media and USB

## Key Files & Configuration

### build.conf
Central configuration file with:
- All component versions
- Feature flags
- APK package lists
- QEMU settings

### Makefile Targets

**Building:**
```bash
make setup              # Initialize build environment
make kernel             # Build kernel only
make rootfs             # Create Alpine rootfs
make disk               # Install packages and configure
make iso                # Create bootable ISO
make all                # Full build
```

**Testing:**
```bash
make boot               # Boot in QEMU (serial)
make boot-gui           # Boot in QEMU (Wayland/GPU)
```

**Cleanup:**
```bash
make clean              # Remove build artifacts
make distclean          # Remove everything
```

## Disk Layout

```
/dev/sda
├── /boot               # Kernel, initramfs
├── /bin → /usr/bin    # Busybox utilities
├── /sbin → /usr/sbin  # System utilities
├── /lib → /usr/lib    # Libraries (musl)
├── /etc                # Configuration
│   ├── sway/           # Sway compositor config
│   ├── xdg/waybar/     # Waybar config
│   └── profile.d/      # Environment setup
├── /usr
│   ├── bin/            # Applications
│   ├── lib/            # Libraries
│   └── share/          # Data files
├── /var/lib/apk/       # APK package database
├── /run/user/1000      # Wayland XDG_RUNTIME_DIR
└── /home/user/         # User home directory
```

## Booting Process

### Serial Boot (`make boot`)
1. QEMU loads kernel + initramfs
2. Initramfs mounts /dev/sda as root
3. OpenRC brings up basic services
4. Serial console login shell

### GUI Boot (`make boot-gui`)
1. QEMU with virtio-gpu device
2. Kernel initializes DRM/framebuffer
3. Wayland display server starts
4. Sway compositor launches
5. Desktop environment ready

## Package Management

All applications installed via APK:

```bash
# Inside running system
apk add package-name
apk update && apk upgrade
apk del package-name
apk info                # List installed packages
```

### Pre-configured Packages

**Core:**
- alpine-base
- busybox (utilities)
- util-linux (system tools)
- e2fsprogs (filesystem)

**Desktop:**
- wayland
- sway
- waybar
- wofi (launcher)
- foot (terminal)

**Utilities:**
- grim (screenshot)
- slurp (selection)
- xdg-desktop-portal-wlr
- OpenRC (init system)

## Planned Applications

Real, production applications to be added:

- **File Manager**: Thunar
- **Text Editor**: gedit / mousepad
- **Terminal**: Foot (already included)
- **Calculator**: Calc
- **System Monitor**: htop
- **Network Manager**: nmcli / nmtui
- **Package Manager GUI**: apk GUI wrapper
- **Settings**: GNOME Control Center or custom
- **Media Player**: VLC
- **Image Viewer**: feh / Eye of MATE
- **PDF Viewer**: Zathura
- **Archive Manager**: xarchiver

## Performance Optimization

- **Static Linking**: Core binaries statically linked
- **Minimal Init**: Alpine OpenRC (fast startup)
- **Memory**: 4GB default in QEMU (configurable)
- **CPU**: Multi-core support (4 cores default)
- **GPU**: Virtio-GPU for QEMU acceleration
- **Disk**: ext4 with modern defaults

## Security Features

- **ASLR**: Enabled
- **DEP/NX**: Enabled
- **Stack Protection**: Enabled
- **musl libc**: Security-focused implementation
- **No Default Passwords**: Users set on first boot
- **Minimal Attack Surface**: Alpine base is very small

## Kernel Configuration Requirements

Critical kernel options for Wayland/Sway:

```
CONFIG_DRM=y                    # Direct Rendering Manager
CONFIG_DRM_VIRTIO_GPU=y         # Virtio GPU support
CONFIG_VIRTIO_GPU=y             # GPU virtualization
CONFIG_FB=y                     # Framebuffer support
CONFIG_DRM_KMS_HELPER=y         # KMS helpers
CONFIG_INPUT_EVDEV=y            # Event input device
```

## Development Workflow

1. **Modify build.conf** to add/remove packages
2. **Update Makefile** if build process changes
3. **Run `make distclean && make all`**
4. **Boot with `make boot-gui`**
5. **Test applications and Sway**

## Extending with Applications

To add a new application:

1. **Source**: Clone repository
2. **Configure**: `./configure --prefix=/usr --sysconfdir=/etc`
3. **Build**: `make`
4. **Install**: Add to disk via APK or manual installation in chroot
5. **Package**: Create APK package for reproducibility

Example (adding custom application):

```bash
# In Makefile disk target, after Sway installation:
cd $(MNT_DIR) && \
    chroot . /sbin/apk add --no-scripts custom-app-package
```

## Troubleshooting

### Boot Issues
- Check kernel DRM support: `grep CONFIG_DRM /boot/config-6.18.33`
- Verify initramfs: `file dist/initramfs.cpio.gz`
- Check disk: `e2fsck dist/nexa.img`

### APK Errors
- Update repositories: `apk update`
- Check network inside chroot
- Verify Alpine mirrors are accessible

### Wayland Issues
- Check XDG_RUNTIME_DIR: `echo $XDG_RUNTIME_DIR`
- Verify DRM device: `ls -la /dev/dri/`
- Check Wayland logs: `journalctl -xe`

## Next Steps

1. ✅ **Makefile Production Implementation** - DONE
2. ✅ **Alpine Linux Integration** - DONE
3. ✅ **Wayland/Sway Configuration** - DONE
4. ⏳ **Kernel Validation** - TODO: Verify DRM/GPU support
5. ⏳ **First Boot Testing** - TODO: Boot and verify
6. ⏳ **Application Integration** - TODO: Add real apps
7. ⏳ **ISO Testing** - TODO: Test ISO boot

## Building & Testing

```bash
# First time: Download Alpine and build
./build.sh build

# Boot and test Wayland
./build.sh boot-gui

# Run CI pipeline
./build.sh ci

# Create ISO for physical media
make iso
```

## References

- **Alpine Linux**: https://alpinelinux.org
- **Wayland**: https://wayland.freedesktop.org
- **Sway**: https://swaywm.org
- **wlroots**: https://gitlab.freedesktop.org/wlroots/wlroots
- **APK Package Manager**: https://wiki.alpinelinux.org/wiki/Alpine_Linux_package_management
