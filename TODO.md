# NexaOS TODO

Working list, in order. Autopilot granted 2026-10-12: build and verify each
item, commit it, move on. `[x]` done and verified · `[~]` built, waiting for
an HP test · `[ ]` not started. Details and history live in BUILD_CHECKLIST.md.

## 1. Current build (Alpine 3.24 + Hyprland)
- [ ] Fix missing app icons on 3.24 (hicolor-icon-theme, gtk-update-icon-cache) and rebuild the ISO
- [ ] Boot-test the ISO in QEMU (Sway path), check Dock/start-menu icons
- [ ] HP test: Hyprland animations/blur, Dock, themes, battery, brightness, lid, Wi-Fi *(needs you)*

## 2. Security
- [ ] Ask for the password before root-level actions (package installs) instead of passwordless doas
- [ ] Optional auto-login: off by default on installed systems (password at boot)
- [ ] Browser: WebKit sandbox on by default
- [ ] Package server: no QEMU-only plain-HTTP default
- [ ] Further hardening (AppArmor, signed boot) — research + plan

## 3. Installer
- [ ] Re-run BIOS + UEFI install tests on Alpine 3.24 (with the password step)
- [ ] Choose your own username during install
- [ ] Install to a real disk and boot it *(needs you)*

## 4. Features
- [ ] Private browsing over Tor (`nexadark` terminal command)
- [ ] Signed updates from GitHub *(needs a GitHub repo + signing key from you)*
- [ ] Remember Wi-Fi networks and browser history across reboots
- [ ] Browser download manager with live download speed
- [ ] System monitor with its own name and a Ctrl+Shift+Esc-style shortcut
- [ ] Phase A extras: translucent app windows, micro-animations, Launchpad
- [ ] Nexa AI: current Claude model, no truncated replies

## 5. Hardware
- [ ] Bluetooth (kernel + bluez + Settings page)
- [ ] Intel SOF audio, NVIDIA (nouveau), Intel Lunar Lake+ (xe) — evaluate

## 6. Release
- [ ] Clean up one-off fix_*.sh / inject_*.sh scripts and old image backups
- [ ] Update BUILDING.md / ARCHITECTURE.md
- [ ] Push to GitHub so CI runs *(needs you: create the repo)*
- [ ] Version number and changelog
