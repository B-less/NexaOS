# NexaOS TODO

Working list, in order. Autopilot granted 2026-10-12: build and verify each
item, commit it, move on. `[x]` done and verified · `[~]` built, waiting for
an HP test · `[ ]` not started. Details and history live in BUILD_CHECKLIST.md.

## 1. Current build (Alpine 3.24 + Hyprland)
- [x] Fix missing app icons on 3.24 (hicolor-icon-theme, gtk-update-icon-cache) and rebuild the ISO
- [x] Boot-test the ISO in QEMU (Sway path): Alpine 3.24.2, firewall, hardening, SSH off, no firmware errors, icons fixed (`scripts/test-live-boot.py`); terminal missing from the Dock (foot renamed in 3.24) — fixed
- [ ] HP test: Hyprland animations/blur, Dock, themes, battery, brightness, lid, Wi-Fi *(needs you)*

## 2. Security
- [x] Ask for the password before root-level actions: software/package installs and the assistant's control tools need it on installed systems (checked against real OpenDoas); power/USB/Settings stay passwordless; the installer helper is removed from installed systems
- [x] Installed systems start at the lock screen (password after every boot); per-user opt-out in Settings > Power; live USB unchanged
- [x] Browser: WebKit sandbox on by default (verified: pages + JavaScript render with web content inside bwrap)
- [x] Package server: the QEMU-only HTTP setting was dead config (only old fix_*.sh used it); apps install from Alpine's signed HTTPS repos
- [ ] Further hardening (AppArmor, signed boot) — research + plan

## 3. Installer
- [ ] Re-run BIOS + UEFI install tests on Alpine 3.24 (with the password step)
- [x] Choose your own username during install (account renamed everywhere, uid stays 1000; system no longer depends on the name "user"); install test now uses "nexa"
- [ ] Install to a real disk and boot it *(needs you)*

## 4. Features
- [ ] Private browsing over Tor (`nexadark` terminal command)
- [ ] Signed updates from GitHub *(needs a GitHub repo + signing key from you)*
- [ ] Remember Wi-Fi networks and browser history across reboots
- [ ] Browser download manager with live download speed
- [x] Nexa Pulse: one app for running apps + processes (search, End task, Force quit) and live CPU/memory/disk/network graphs; Ctrl+Shift+Esc in Sway and Hyprland; old Tasks/System Monitor hidden (checked in headless Sway: `scripts/test-app-headless.sh`)
- [ ] Phase A extras: translucent app windows, micro-animations, Launchpad
- [x] Nexa AI: claude-opus-5-5, max_tokens 16000, server-side refusal fallback, refusal/cut-off handling (tested against a fake API)

## 5. Hardware
- [ ] Bluetooth (kernel + bluez + Settings page)
- [ ] Intel SOF audio, NVIDIA (nouveau), Intel Lunar Lake+ (xe) — evaluate

## 6. Release
- [ ] Clean up one-off fix_*.sh / inject_*.sh scripts and old image backups
- [ ] Update BUILDING.md / ARCHITECTURE.md
- [ ] Push to GitHub so CI runs *(needs you: create the repo)*
- [ ] Version number and changelog
