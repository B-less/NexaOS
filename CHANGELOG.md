# Changelog

## 3.2.0 — 2026-10-12

Everything since the 3.1.0 import. Tested in QEMU (BIOS and UEFI); the
first real-hardware boot (HP EliteBook 840 G6) worked for USB boot, Wi-Fi
and sound with an earlier build. Bluetooth and Hyprland still need a
real-hardware test.

### Base
- Alpine 3.24.2 (3.19 is end-of-life); Linux 6.18.33, monolithic, with
  real-hardware drivers: Intel and AMD graphics, Intel/Realtek/MediaTek/
  Atheros/Broadcom Wi-Fi, USB Bluetooth, WireGuard; GPU firmware in the
  initramfs. (NVIDIA and Intel Lunar Lake+ graphics use the basic display
  driver for now.)
- Hybrid ISO for Rufus/dd: BIOS and UEFI, silent GRUB (Esc for the menu)
  with diagnostic entries.
- Boot animation: a hexagon builds itself, a light beam writes NEXA, the
  hexagon becomes the progress loader.

### Desktop
- Hyprland session with animations, blur and rounded corners on machines
  with a real GPU; Sway otherwise, or if Hyprland fails to start.
- Top bar, start menu (Super+D), Control Centre (Super+N), Dock with
  magnification, Launchpad (Super+Space), notifications with Do Not Disturb.
- Five themes (Horizon, Nebula, Aurora, Eclipse, Daybreak) that every app
  follows live; glass (see-through) app windows; adjustable bar/panel
  transparency; wallpaper gallery and your own pictures.
- Display fixes across the apps: dialogs, cursor, minimize/maximize, sizing.

### Apps
- **Nexa Pulse** (Ctrl+Shift+Esc): open apps and processes with search, End
  task and Force quit; live CPU, memory, disk and network graphs. Replaces
  Nexa Tasks and Nexa System Monitor.
- **Nexa Browser**: history with address-bar suggestions, saved sign-ins,
  third-party cookies blocked, download manager with live speed and time
  left, private windows (Ctrl+Shift+N), Tor windows (`nexadark`).
- **Settings**: Bluetooth (pair, connect, forget), VPN (WireGuard files),
  Personalization, password-at-startup switch.
- **NexaOS Assistant**: current Claude model, handles refusals and cut-off
  replies.

### Security
- Password needed for package installs and the assistant's system actions
  on installed systems; installed systems start at the lock screen.
- Browser pages run in a sandbox; WebRTC and DNS prefetching off.
- Firewall drops unsolicited inbound traffic; kernel and network sysctl
  hardening; SSH off.
- Fixed a hole where a `.nexa` package could replace root helpers or system
  commands (package names were used as paths by a root helper).
- VPN files that would run commands as root are refused.
- Plan for the next steps: docs/HARDENING.md.

### Installer
- Choose your username and password; disk installs boot via GRUB on both
  BIOS and UEFI; Wi-Fi networks from the live session carry over.

### Build and tests
- `make refresh` (rebuild image and ISO in ~5 min), `make test` (15 suites),
  QEMU live-boot and install-cycle tests, headless app screenshots.
- Old one-off scripts moved to `legacy/`.
