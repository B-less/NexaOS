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
- [x] Further hardening — research + plan written: docs/HARDENING.md (kernel self-protection → Landlock for root helpers → AppArmor for internet-facing apps → own-key Secure Boot → disk encryption)

## 3. Installer
- [x] BIOS + UEFI install tests pass on Alpine 3.24 with username "nexa" + password (BIOS boot in QEMU takes ~95 s: SeaBIOS reads the 37 MB initramfs slowly; harness wait raised to 240 s)
- [x] Choose your own username during install (account renamed everywhere, uid stays 1000; system no longer depends on the name "user"); install test now uses "nexa"
- [ ] Install to a real disk and boot it *(needs you)*

## 4. Features
- [x] Private browsing over Tor: `nexadark` (or Tor window button / start menu) opens a private window through a per-user Tor on 127.0.0.1:9150; verified check.torproject.org says "configured to use Tor" with normal DNS switched off. Honest note in the window: not Tor Browser (fingerprinting)
- [x] Private windows (Ctrl+Shift+N): nothing saved
- [x] VPN: WireGuard in the kernel + Settings > VPN (import .conf, on/off, remove; files that run commands refused; VPN DNS applied and restored). Tested with real wg-quick against a peer in another network namespace
- [ ] Signed updates from GitHub *(needs a GitHub repo + signing key from you)*
- [x] Remember Wi-Fi networks and browser history across reboots: browser history (search, URL-bar suggestions, clear) and sign-in cookies are saved; Wi-Fi was already remembered by iwd on installed systems, and networks joined in the live session carry over at install. The live USB itself forgets everything at shutdown (RAM overlay) — persistence there would need a storage partition
- [x] Browser download manager: files go to ~/Downloads (safe, unique names), panel shows size, live speed and time left, Cancel / Show; non-viewable files and attachments download instead of failing
- [x] Nexa Pulse: one app for running apps + processes (search, End task, Force quit) and live CPU/memory/disk/network graphs; Ctrl+Shift+Esc in Sway and Hyprland; old Tasks/System Monitor hidden (checked in headless Sway: `scripts/test-app-headless.sh`)
- [x] Phase A extras: glass (translucent, blurred on Hyprland) app windows with a Settings switch, eased button/row transitions, Launchpad (Super+Space or the Dock: full-screen app grid with type-to-search)
- [x] Nexa AI: claude-opus-5-5, max_tokens 16000, server-side refusal fallback, refusal/cut-off handling (tested against a fake API)

## 5. Hardware
- [~] Bluetooth: kernel + bluez + Bluetooth audio + bt-probe firmware re-bind + Settings page with pairing agent (tested against a mock BlueZ; needs a real adapter: HP test)
- [ ] Intel SOF audio, NVIDIA (nouveau), Intel Lunar Lake+ (xe) — evaluated, not enabled yet: built-in GPU drivers load firmware before the root filesystem exists; nouveau (Turing+ GSP) and xe firmware are too big for the initramfs, and a driver that takes the screen over from simpledrm and then fails leaves it black. Needs either firmware-in-initramfs per GPU family or loadable modules. SOF: only needed on newer Intel laptops whose sound doesn't work with HDA (840 G6 sound works).

## 6. Release
- [x] Clean-up: one-off scripts moved to legacy/ (git mv); old image backups and logs moved to attic/ (git-ignored, 272 MB, delete when you like)
- [x] BUILDING.md / ARCHITECTURE.md rewritten for the current system
- [ ] Push to GitHub so CI runs *(needs you: create the repo)*
- [x] Version 3.2.0 and CHANGELOG.md
