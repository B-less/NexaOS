# NexaOS Release Checklist

Status as of 2026-10-11. Legend:

- `[x]` done — confirmed from files/config in the repo, or boot-tested in an earlier session (marked *tested earlier*)
- `[~]` partly done, or done but the result was never recorded — needs a re-check
- `[ ]` not done

**Progress: 50 of 93 done, 23 partial → ~66%** (partial counts as half)

---

## 1. Kernel (6.18.33, monolithic, `CONFIG_MODULES` off)

- [x] Kernel source in `kernel/linux-6.18.33/`, config tracked as `kernel.config.6.18.33`
- [x] Graphics/input: `DRM`, `DRM_VIRTIO_GPU`, `DRM_SIMPLEDRM`, `FB`, `INPUT_EVDEV`
- [x] Filesystems: `EXT4_FS`, `TMPFS`, `VFAT_FS`
- [x] Install targets: `EFI`, `EFI_STUB`, `EFI_PARTITION`, `SATA_AHCI`, `BLK_DEV_NVME`, `USB_XHCI_HCD`, `USB_STORAGE`
- [x] WiFi/audio: `IWLWIFI`, `MAC80211_HWSIM`, `SND_HDA_INTEL`
- [x] `dist/bzImage` built (10 MB)
- [x] Kernel boots under UEFI (OVMF) with simpledrm framebuffer — *tested earlier*

## 2. Build system

- [x] Makefile targets: `kernel rootfs initramfs disk gui apps iso boot boot-gui docker test all`
- [x] Docker build environment (`Dockerfile`, `make docker`)
- [x] Build settings centralised in `build.conf`
- [~] CI workflow exists (`.github/workflows/ci.yml`) but has never run — no git remote yet

## 3. Root filesystem (Alpine 3.19)

- [x] Alpine rootfs provisioned (`scripts/provision-rootfs.sh`)
- [x] `/etc/passwd`, `group`, `shadow` forced to `root:root`; passwd/group mode 644 (dbus needs this)
- [x] Sway, Waybar, Wofi, foot, seatd, dbus installed
- [x] Intel WiFi firmware (`linux-firmware-other`), iwd, PipeWire, GStreamer, WebKitGTK, GRUB tools in package list
- [x] `dist/initramfs.cpio.gz` built (0.9 MB)
- [x] `dist/nexa.img` built (2 GB ext4)

## 4. Boot (QEMU)

- [x] Serial boot to login shell — *tested earlier*
- [x] GUI boot to Sway + Waybar desktop — *tested earlier*
- [x] Framebuffer desktop fallback (`nexa-fbdesktop`) when no GPU driver — *tested earlier*
- [x] Boot splash, lock screen

## 5. Desktop & apps

- [x] 14 C framebuffer apps (`gui/apps/`: about, archive, audio, calc, clock, csv_view, editor, files, image, md_view, props, sysmon, term, todo)
- [x] File manager, text editor, browser (WebKitGTK), settings
- [x] Music player, image viewer, PDF viewer, notes
- [x] Task manager, system monitor, volume mixer, power menu
- [x] Screenshot, colour picker, character map, clipboard, archive manager
- [x] Desktop icons, automount for removable drives
- [x] Nexa AI app
- [x] Shared Python library `common/nexa_common` with host tests (`common/tests/`)

## 6. Networking

- [x] Wired DHCP + ping in QEMU — *tested earlier*
- [x] WiFi backend over iwd D-Bus, no root needed (`common/nexa_common/wifi.py`, mock-iwd tests)
- [x] Network settings app
- [x] `wifi-probe` service re-probes devices once firmware is available
- [x] WiFi on real hardware: HP EliteBook 840 G6 (Intel AC 9560) connects via the Network app (2026-10-11, after the hwsim fix)

## 7. Packages & software

- [x] `.nexa` package installer (`installer/nexa_installer`)
- [x] Software center
- [x] Online repo with a test package (`online_repo/test-app`)
- [~] Repo is local only — no hosted package server yet

## 8. Live ISO

- [x] `scripts/build-iso.sh` (isolinux + xorriso)
- [x] `dist/NexaOS-3.1.0.iso` built (2.1 GB, BIOS boot)
- [x] Hybrid ISO with UEFI GRUB + `nexa.live` USB search (2026-10-11). Test ISO with a stub nexa.img boots: BIOS CD, BIOS USB, UEFI USB (Rufus DD), UEFI FAT32 USB (Rufus ISO mode), UEFI SATA CD. Needs `make iso` with the real image
- [ ] Rebuild the ISO — it (09-26) is older than `nexa.img` and `initramfs` (09-27)
- [x] Flash with Rufus and boot a real PC (Secure Boot off: GRUB is unsigned) — HP EliteBook 840 G6
- [x] Installer never offers the live USB stick as a target (`/mnt/nexa-live`, unit-tested)

## 9. OS installer (`installer-os/nexa_setup`)

- [x] Graphical setup app + privileged helper
- [x] Disk install: GPT with BIOS-boot + 320 MB ESP + root (`common/nexa_common/diskinstall.py`)
- [x] GRUB for both BIOS (`i386-pc`) and UEFI (`x86_64-efi --removable`)
- [x] Automated install-and-reboot test (`scripts/test-install-cycle.py`, `--uefi` option)
- [x] Install test PASS in BIOS mode (2026-10-11): installed disk boots on its own via GRUB
- [x] Install test PASS in UEFI mode (2026-10-11, `--uefi`): installed disk boots on its own via OVMF → GRUB → login
- [~] Installer asks for a password and sets it for `user` and `root` (code + unit tests done; needs `make disk`, then the install test, which now logs in with it)
- [ ] Installer lets the user pick a username (`user` is hard-coded in the session launcher, doas rules, sway config)
- [~] System files in the image owned by root. They were uid 1000 (`user`), so `user` could replace the passwordless doas helpers and get root. Fixed in `build-disk-image.sh`; needs `make disk` and a check

## 10. Security (found 2026-10-11 review)

- [x] `.nexa` packages can't overwrite or delete what they didn't install (root helpers, system code, system commands), can't leak files through `icon`, can't inject desktop-entry lines (2026-10-11, `common/tests/test_package_helper.py`)
- [x] `make test` runs the real host test suites (it only printed "passed" before)
- [ ] doas helpers are all `nopass`; once accounts have passwords, root-equivalent ones (package installs) should ask for it
- [~] sshd off by default (opt-in with `rc-update add sshd`) — needs rebuild
- [~] Firewall: nftables, incoming blocked except replies; kernel netfilter enabled — needs kernel + image rebuild
- [~] Kernel hardening sysctls (kptr/dmesg restrict, link protections, no redirects, syncookies) — needs rebuild
- [ ] Desktop auto-logs in as `user` on boot, so the password only guards SSH/console/lock screen
- [~] Alpine 3.19 is end-of-life: moving to 3.24.2 (see Roadmap)
- [ ] Package server URL is QEMU-only (`PKG_SERVER_URL=http://10.0.2.2:8080`, plain HTTP)

## 11. Real-hardware drivers (kernel is monolithic, so each needs a rebuild)

- [~] (config done 2026-10-11, needs kernel build + real-PC test) GPU: no `DRM_I915`/`DRM_XE`/`DRM_AMDGPU`/`DRM_NOUVEAU` — real PCs get only the UEFI framebuffer (fixed resolution, no brightness, CPU rendering); BIOS-mode boots may get no graphics
- [~] (config done) Ethernet: no `R8169` (Realtek, the most common), no `IGB`
- [~] (config done; Broadcom/Realtek/MediaTek/Atheros firmware added) WiFi: only Intel (`IWLWIFI`) and `ATH9K`; no Realtek `RTW88/RTW89`, MediaTek `MT7921E/MT7925E`, Atheros `ATH10K/11K/12K`, Broadcom `BRCMFMAC`
- [~] (config done, incl. `PINCTRL`, `PCI_MSI` which was also off) Laptop touchpads: no `I2C_HID_ACPI`, `HID_MULTITOUCH`, `I2C_DESIGNWARE_*`
- [ ] Still missing: Bluetooth (needs bluez + UI too), Intel SOF audio (firmware at probe), NVIDIA (`nouveau`), Intel Lunar Lake+ (`xe`). Done: webcam, backlight, card readers, VMD, laptop hotkeys. Was: no Bluetooth, webcam (`MEDIA_SUPPORT`), backlight (`BACKLIGHT_CLASS_DEVICE`), Intel SOF audio
- [~] i915/amdgpu firmware copied into the initramfs (needs rebuild). Firmware is from 20240312, so GPUs newer than early 2024 may fail until Alpine is upgraded

## 12. Apps

- [ ] Nexa AI: default model `claude-sonnet-5` is a generation old (current: `claude-opus-5-5` / `claude-sonnet-5-5`), and `max_tokens=1024` truncates replies now that thinking is on by default

## 14. Laptop basics (batch 1, 2026-10-11) — code done, needs rebuild + HP test

- [~] Clock in the top bar (was hidden: no /etc/localtime; now UTC default)
- [~] Battery + brightness in the top bar; low-battery warnings at 15% / 5%
- [~] Brightness keys (brightnessctl), touchpad tap-to-click + natural scroll
- [~] Lid close locks and suspends

## 15. Roadmap (user priorities, 2026-10-11)

- [~] New desktop (2026-10-11, needs rebuild + HP test): top bar redesign with Font Awesome icons; start menu (Super+D) with search, pinned apps, power; Control Centre (Super+N) with toggles, volume/brightness, battery, notifications, Do Not Disturb; 5 themes (Horizon, Nebula, Aurora, Eclipse, Daybreak) with transparency. Checked in a headless Sway container (screenshots), not on the HP yet
- [~] Settings > Personalization: theme cards, transparency slider, wallpaper gallery + upload (needs HP test)
- [~] Alpine 3.19 → 3.24.2 (all packages present; firmware now zstd → kernel option; Intel Wi-Fi firmware moved to linux-firmware-intel). Needs kernel + full rebuild and HP test
- [ ] Private browsing mode over Tor (terminal command `nexadark`) and optional VPN
- [ ] Signed updates from a GitHub repository (needs repo + signing key)
- [ ] Remember Wi-Fi networks and browser history across reboots on installed systems
- [ ] Browser download manager with live download speed
- [ ] System monitor app with its own name and keyboard shortcut (like Ctrl+Shift+Esc)
- [~] macOS-style polish phase A (needs rebuild + HP test): apps follow the theme with live switching; start menu/Control Centre fade+slide animation; Dock with magnification, running dots, launch bounce; desktop icons removed. Checked in headless sway
- [ ] Phase A remaining: translucent app windows, micro-animations in apps, Launchpad
- [~] Phase B: Hyprland session on GPU machines (window pop-in/out, workspace slide, blur behind bar/Dock/panels, rounded corners, shadows, 3-finger workspace swipe, themed gradient borders); Sway on other machines and as automatic fallback. Config passes `Hyprland --verify-config` (0.54.3); never run for real yet (no GPU here) — first run is the HP
- [ ] Browser: turn WebKit's sandbox on by default (currently off, untested with real sites)
- [ ] Further hardening (AppArmor, signed boot, sandboxing)

## 13. Real hardware & release

- [x] Boot the live ISO/USB on a real machine (HP EliteBook 840 G6, UEFI, Rufus; sound works)
- [ ] Install to a real disk and boot it (BIOS and UEFI)
- [ ] Real GPU, audio, touchpad check
- [ ] Remove one-off `fix_*.sh` / `inject_*.sh` scripts and stale `nexa.img.before-*` backups
- [ ] Update `BUILDING.md` / `ARCHITECTURE.md` to match the final build
- [ ] Push to a remote so CI runs
- [~] Release version / notes (ISO is named 3.1.0, no changelog)

---

## Next steps (in order)

1. `make disk`, re-run both install tests, check image ownership
2. Username choice in the installer (optional)
3. Add UEFI boot to the ISO, then rebuild it
4. Real-hardware test from USB
5. Cleanup and docs
