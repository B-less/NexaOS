# NexaOS TODO

Working list, in order. `[x]` done and verified · `[~]` partly built (what's
missing is said on the line) · `[ ]` not started. Details and history live in
BUILD_CHECKLIST.md. Last checked against the code: 2026-10-15.

## 1. Current build (Alpine 3.24 + Hyprland)
- [x] Fix missing app icons on 3.24 (hicolor-icon-theme, gtk-update-icon-cache) and rebuild the ISO
- [x] Boot-test the ISO in QEMU (Sway path): Alpine 3.24.2, firewall, hardening, SSH off, no firmware errors, icons fixed (`scripts/test-live-boot.py`); terminal missing from the Dock (foot renamed in 3.24) — fixed
- [~] HP test: first run 2026-10-14 (the assistant: its folder, path and network problems were fixed). Still to check: Hyprland animations/blur, Dock, themes, battery, brightness, lid, Wi-Fi *(needs you)*

## 2. Security
- [x] Ask for the password before root-level actions: software/package installs and the assistant's control tools need it on installed systems (checked against real OpenDoas); power/USB/Settings stay passwordless; the installer helper is removed from installed systems
- [x] Installed systems start at the lock screen (password after every boot); per-user opt-out in Settings > Power; live USB unchanged
- [x] Browser: WebKit sandbox on by default (verified: pages + JavaScript render with web content inside bwrap)
- [x] Package server: the QEMU-only HTTP setting was dead config (only old fix_*.sh used it); apps install from Alpine's signed HTTPS repos
- [x] No root shell on the screen: tty1 used to open a root shell with no login (Ctrl+Alt+F1); now the desktop owns tty1 and tty2 asks for a login (verified in QEMU)
- [x] Further hardening — research + plan written: docs/HARDENING.md (kernel self-protection → Landlock for root helpers → AppArmor for internet-facing apps → own-key Secure Boot → disk encryption)

- [x] Lock screen: unlocks again (PAM couldn't read /etc/shadow; live USB ignored the empty Enter), new look (blur, clock, theme colours); verified in QEMU: empty Enter unlocks the live USB, installed system accepts the right password and refuses a wrong one
- [x] Dragging windows: Nexa apps drag by their titlebars (verified with a virtual mouse); the terminal now has its own title bar and drags; Sway gives a title bar to any other app without one

- [x] Persistent live USB: Settings > System > "Keep my files on this USB" adds a NEXA_PERSIST partition after the ISO (DD-mode sticks); files, settings, Wi-Fi and apps survive reboots; home survives writing a newer ISO; "don't load saved changes" boot entry. QEMU test (3 boots, 8 GB stick): PASS

## 3. Installer
- [x] BIOS + UEFI install tests pass on Alpine 3.24 with username "nexa" + password (UEFI re-run on the 3.2.0 ISO: PASS) (BIOS boot in QEMU takes ~95 s: SeaBIOS reads the 37 MB initramfs slowly; harness wait raised to 240 s)
- [x] Choose your own username during install (account renamed everywhere, uid stays 1000; system no longer depends on the name "user"); install test now uses "nexa"
- [ ] Install to a real disk and boot it *(needs you)*

## 4. Features
- [x] Private browsing over Tor: `nexadark` (or Tor window button / start menu) opens a private window through a per-user Tor on 127.0.0.1:9150; verified check.torproject.org says "configured to use Tor" with normal DNS switched off. Honest note in the window: not Tor Browser (fingerprinting)
- [x] Private windows (Ctrl+Shift+N): nothing saved
- [x] VPN: WireGuard in the kernel + Settings > VPN (import .conf, on/off, remove; files that run commands refused; VPN DNS applied and restored). Tested with real wg-quick against a peer in another network namespace
- [ ] Signed updates from GitHub: the repo and the signing key exist now; still to build: the release signing + update tool, and a public place to download releases from (the repo is private)
- [x] Remember Wi-Fi networks and browser history across reboots: browser history (search, URL-bar suggestions, clear) and sign-in cookies are saved; Wi-Fi was already remembered by iwd on installed systems, and networks joined in the live session carry over at install. The live USB itself forgets everything at shutdown (RAM overlay) — persistence there would need a storage partition
- [x] Browser download manager: files go to ~/Downloads (safe, unique names), panel shows size, live speed and time left, Cancel / Show; non-viewable files and attachments download instead of failing
- [x] Nexa Pulse: one app for running apps + processes (search, End task, Force quit) and live CPU/memory/disk/network graphs; Ctrl+Shift+Esc in Sway and Hyprland; old Tasks/System Monitor hidden (checked in headless Sway: `scripts/test-app-headless.sh`)
- [x] Phase A extras: glass (translucent, blurred on Hyprland) app windows with a Settings switch, eased button/row transitions, Launchpad (Super+Space or the Dock: full-screen app grid with type-to-search)
- [x] Nexa AI: claude-opus-5-5, max_tokens 16000, server-side refusal fallback, refusal/cut-off handling (tested against a fake API)
- [x] .nexa apps: nexa-pack (Rust: build, Ed25519-sign, verify, reproducible), Nexa Installer, the first app (Video Player, C + GStreamer, signed, installs through the broker), rules for writing apps (docs/WRITING_NEXA_APPS.md, AGENTS.md); theme colours reach every app
- [x] Faster boot: login prompt at ~18 s instead of ~38 s in QEMU (DHCP no longer holds up the desktop)
- [x] Files: multi-select (click, Ctrl/Shift+click, drag a box, Ctrl+A), drag and drop (move; Ctrl copies), right-click menus for items and empty space, Cut/Copy/Paste, Duplicate, Rename (F2), Delete = Trash with Ctrl+Z undo, Trash view (Restore, Empty), Back/Forward (+ mouse buttons), Properties with folder sizes, Ctrl+scroll zoom, sortable columns, type-to-find, live refresh, Open Terminal Here, new window (middle-click). Every change runs in nexa-fileops (Rust): never overwrites, copy_file_range, freedesktop Trash incl. USB drives' own .Trash-<uid>, Landlock per request (`make test-files-app`)
- [ ] More of our own apps (.nexa, C/Rust): Photos, Music, Backup, Mail
- [ ] Smart power: power-saving mode on battery, battery health, time-left estimate

## 4b. Next-gen ideas, made practical (from the "next-generation OS" article)
- [ ] Per-app internet control ("network-level sandboxing"): choose per app: normal, VPN only, Tor only, or no internet (network namespaces + nftables per app)
- [ ] Reopen your windows after a reboot or power cut ("orthogonal persistence", practical form): session save/restore + app autosave
- [ ] Safe updates with rollback ("zero-downtime patching", practical form): update into a new system layer, keep the previous one, one-click undo
- [x] Per-app permissions ("capability sandboxing"): installed .nexa apps start through nexa-run (Rust), which confines them with Landlock to what they declared: their own folders, the files.* folders they asked for, a file opened with them, TCP only with "network", the camera only with "camera"; the person can switch permissions off (~/.config/nexa/app-permissions.json); `nexa-run --explain <id>`. Not enforceable with Landlock yet: microphone, notifications, location, background, UDP
- [x] Privacy Center (Settings > Privacy): each installed app with who signed it and what it may use, a switch per permission (takes effect at the app's next start, enforced by nexa-run; permissions Landlock can't enforce yet are marked and their switch is disabled)
- [~] Smart search ("semantic file system", practical form): the assistant finds files by name, recent changes and what takes up space (nexa-tools); still missing: contents, tags and dates ("last Friday"), a local index, search from Launchpad
- [ ] Self-healing desktop parts ("restart the failed module"): bar, Dock, notifications and Wi-Fi restart themselves if they crash
- Not planned: microkernel, neural scheduler, NPU fabric, ambient cloud compute, phone convergence (research-OS scale, not possible on a Linux base)

## 4c. Nexa Mind (on-device AI, docs/NEXA_MIND.md)
- [x] M1: nexa-mindd (Rust) — sandboxed service supervising llama.cpp, verified model downloads, local back-end for the assistant with tool calls; Settings download + backend choice. End-to-end test with the real model passes
- [ ] M2: kernel PSI / uclamp / RAPL / THP + cgroup for the model (needs a kernel rebuild)
- [~] M2 (kernel link): nexa-kerneld (Rust) — PSI triggers, crash events (netlink process connector), OOM kills (/dev/kmsg), RAPL/battery watts per program; CPU floor (uclamp) for the app in front via focus-watch; the AI's own cgroup (yields while you work, capped under pressure); assistant tools system_pressure / energy_users / kernel_events. Kernel options set in kernel.config.6.18.33 — **needs `make kernel` + the next ISO build**; Pulse view to do
- [x] M3 (files): nexa-tools (Rust) — Landlock per call, write/move/copy with undo, Trash, tidy (plan then apply), duplicates, activity log; Hexa action cards with Undo + Activity timeline in the assistant and Settings
- [x] M3 (web + system): web search/read/weather/Wikipedia/download (private addresses refused, network per call), date/time, battery, busiest programs, close own programs, exact maths + units, space, recent files; untrusted-content guard
- [x] System broker nexa-brokerd (Rust, docs/SYSTEM_BROKER.md): every root action (power, CPU, brightness, time zone, hostname, Bluetooth, assistant settings, services + at boot, updates, packages, .nexa apps, VPN, drives, passwords, users) checked, password where needed, Landlock per action, audited; replaces the Python root helpers (assistant control, power, software, .nexa install, mount, most of settings)
- [ ] M3 (rest): documents (PDF/Office)
- [x] Settings > Users (people, administrators, change my password, add / remove a person with their files kept), Updates (check, see what changes, update now) and Startup (switch services on/off at startup; the system's own services aren't listed): all through the broker, password where it needs one
- [ ] Move the OS installer and persistence partition helpers last
- [ ] M4-M6: private index, screen OCR, offline voice
- [ ] Download the model on the HP and time real answers there

## 5. Hardware
- [~] Bluetooth: kernel + bluez + Bluetooth audio + bt-probe firmware re-bind + Settings page with pairing agent (tested against a mock BlueZ; needs a real adapter: HP test)
- [ ] Intel SOF audio, NVIDIA (nouveau), Intel Lunar Lake+ (xe) — evaluated, not enabled yet: built-in GPU drivers load firmware before the root filesystem exists; nouveau (Turing+ GSP) and xe firmware are too big for the initramfs, and a driver that takes the screen over from simpledrm and then fails leaves it black. Needs either firmware-in-initramfs per GPU family or loadable modules. SOF: only needed on newer Intel laptops whose sound doesn't work with HDA (840 G6 sound works).

## 6. Release
- [x] Clean-up: one-off scripts moved to legacy/ (git mv); old image backups and logs moved to attic/ (git-ignored, 272 MB, delete when you like)
- [x] BUILDING.md / ARCHITECTURE.md rewritten for the current system
- [x] On GitHub (B-less/NexaOS, private) with CI on every push: Rust tests + static build, then all 17 Python suites; first run green (2026-10-15)
- [x] Lighter rebuilds: build tools saved in nexa-build / nexa-rust-apps images (one-time 29 MB); a full `make refresh` now downloads ~3 MB (was ~37 MB) and takes ~7 min (measured)
- [x] Version 3.2.0 and CHANGELOG.md
