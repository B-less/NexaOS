# NexaOS: design and roadmap ("the dream OS")

Written 2026-10-13. NexaOS 3.2.0 is a working Alpine-based desktop. This is
the plan to make it an OS with its own identity, a private offline
assistant at its core, and a complete set of settings and apps, built and
tested in phases. Every phase ends with real tests (unit tests, headless
screenshots, QEMU boots) and a commit; the HP EliteBook is the hardware
check.

---

## 1. Our own identity (no borrowed UI)

Today several parts follow other systems closely: a magnifying Dock and a
"Launchpad" (macOS), a "Control Centre" (macOS), a Start menu (Windows),
window buttons in the Windows position. Layout ideas themselves are not
owned by anyone, but **names and recognisable trade dress are**.
"Launchpad", "Control Center", "Mission Control", "Spotlight" and "Start"
are trademarks, and copying them makes NexaOS look like an imitation. (This
is a design decision, not legal advice.)

### The Nexa design language: "Hexa"

Built from the hexagon in the boot animation and the constellation
wallpapers: shapes nobody else uses.

| Element | Today | Nexa design |
|---|---|---|
| App launcher | Start menu + Launchpad (two things) | **Nexus**: one full-screen launcher. Apps in a hexagonal honeycomb grid; search bar is also the assistant ("open my photos from Friday", "2+2", "turn on VPN") |
| Dock | macOS-style magnifying dock | **Orbit**: a floating rail; app icons sit in hex frames, running apps get a light ring that pulses with activity; no magnification |
| Quick settings | "Control Centre" panel | **Deck**: stacked cards that slide out from the bar's right edge (Wi-Fi, Bluetooth, VPN, focus, brightness, volume, now playing) |
| Window overview | none | **Constellation**: all windows as nodes on a star map, grouped by app, with lines between windows of the same project/workspace |
| Window buttons | Windows-style boxes | **Hexa buttons**: hexagon outlines with a small symbol (maximise is a hexagon in a hexagon), filled with the accent on hover, red for close (done in 3.3) |
| Notifications | mako popups | **Signals**: notification capsules that dock into the bar's edge; grouped by app; actions inline |
| Lock screen | swaylock | **Nexa Lock** (own design: hexagon clock face, time arc, battery and notifications count; still a secure session-lock client underneath) |
| Typography | system default | **Inter** for UI, **JetBrains Mono** for code (both OFL-licensed) with a defined type scale |
| Icons | own SVGs | Complete own icon set in the hex-tile style, including the settings pages |
| Motion | per-widget | One motion spec: 160 ms ease-out for state, 240 ms "settle" curve for panels, reduced-motion setting honoured everywhere |

Design tokens (colours, radii, spacing, type, motion) live in one place
(`nexa_common/design.py`) and generate the GTK CSS, the bar, the lock
screen and the compositor config, so the whole system changes together.

---

## 2. What makes NexaOS different

1. **Nexa Mind, a private assistant that works offline.**
   - Local model through `llama.cpp` (in Alpine): a small model (1–3 B
     parameters, ~1–2 GB, downloaded from Settings, never preinstalled) runs
     on the CPU; Claude remains an optional online mode for harder tasks.
   - Voice: `whisper.cpp` speech-to-text and `espeak-ng` voice, offline.
   - System actions with explicit permission (the existing tool layer).
   - "Ask about this screen" and **copy text from anything** (`tesseract` OCR).
   - Local index of your files so it can answer "where's the invoice from
     March?" Nothing leaves the machine.
2. **Privacy Center.** One page that shows and controls, per app: internet
   (normal / VPN only / Tor only / offline), camera, microphone, location,
   files; Tor windows and VPN built in.
3. **Time Rewind.** System snapshots before every update and on a schedule;
   one click to roll back the system, separate from your files.
4. **Nexa Go.** The persistent live USB (done in 3.2): your whole system in
   your pocket.
5. **Pulse** (done): task manager and system monitor in one.
6. **Focus modes.** Work / Study / Gaming / Night: each sets notifications,
   wallpaper, theme, which apps may notify, and power profile.

---

## 3. Settings, page by page

Each page has a search entry in Settings' own search, and the assistant can
open it ("open display settings").

| Page | Contents | Status |
|---|---|---|
| **Wi-Fi & Network** | networks, saved networks, hidden network, IP/DNS, Ethernet, airplane mode, hotspot | partly (Network app) |
| **Bluetooth** | power, pair, connect, forget, battery level of devices | done |
| **VPN** | WireGuard import, on/off, per-app routing (Privacy) | done (per-app: new) |
| **Display** | resolution, refresh rate, scaling, rotation, arrangement of monitors, mirror/extend, night light, brightness | new |
| **Sound** | output/input device, volume, balance, per-app volume, alert sound, test speakers | new |
| **Power & Battery** | battery health and graph, power profile (saver/balanced/performance), lid action, idle times, low-battery warnings, startup lock | partly |
| **Personalization** | themes, accent colour, wallpaper, glass, fonts and size, icon size, motion | partly |
| **Desktop** | Orbit (position, size, auto-hide, pinned apps), bar layout, Nexus behaviour, hot corners, workspaces | new |
| **Notifications & Focus** | per-app notifications, do-not-disturb schedule, focus modes | new |
| **Apps** | installed apps, default apps (browser, mail, video, photos), startup apps, app permissions | new |
| **Privacy & Security** | Privacy Center, firewall, Tor, lock screen, password, app sandboxing, location, camera/mic indicators | new |
| **Users** | your account, picture, password, add user, guest session | new |
| **Keyboard** | layouts, repeat rate, shortcuts editor | partly |
| **Mouse & Touchpad** | speed, natural scrolling, tap-to-click, gestures | new |
| **Date & Time** | time zone, automatic time (NTP), 24-hour clock | partly |
| **Language & Region** | language, formats, input methods | new |
| **Accessibility** | larger text, high contrast, reduce motion, screen reader (Orca), magnifier, sticky keys, cursor size | new |
| **Storage** | usage by category, clean-up, drives, USB storage | new |
| **Updates** | check, install, auto-update, history, rollback (Time Rewind) | new |
| **Backup** | Nexa Backup settings (see apps) | new |
| **Nexa Go** | live USB persistence | done |
| **Nexa Mind** | offline model download/remove, online (Claude) on/off, voice, what it may do | partly |
| **Developer** | shell, logs, Pulse, SSH, Flatpak/Distrobox, debug overlay | new |
| **About** | version, hardware, licences, "check for updates" | partly |

---

## 4. Apps

Our own (simple, fast, consistent):

| App | What | Built on |
|---|---|---|
| **Nexa Video** | player with library, subtitles, resume, picture-in-picture | GStreamer |
| **Nexa Photos** | library by date/place, albums, basic edits, "find text in photos" (OCR) | GdkPixbuf, tesseract |
| **Nexa Mail** | Gmail/Outlook/IMAP accounts, unified inbox, offline reading, Nexa Mind summaries (local) | IMAP/SMTP |
| **Nexa Calendar** | CalDAV/Google/Outlook calendars, reminders | CalDAV |
| **Nexa Backup** | home folder to USB or folder, schedule, restore single files | rsync + snapshots |
| **Nexa Browser** | tabs, password manager, reader mode, web apps | WebKit |

From Alpine, one click in Nexa Software's **Featured** page: Firefox,
Chromium, Thunderbird, LibreOffice, VLC, GIMP, Inkscape, Krita, Blender,
OBS, Audacity, KeePassXC. Then **Flatpak** for Chrome, Spotify, Zoom,
Discord, Steam.

---

## 5. Quality: debugging what exists

Static analysis (ruff, all 158 Python files): clean except six harmless
warnings in one test. The real bugs are behavioural, like the lock screen
that could not unlock and the terminal that could not be dragged. So:

1. **Every app, every page, screenshotted** in headless Sway, light and dark
   theme, at 100 % and 150 % scaling; app logs checked for errors.
2. **Click-through tests** with the virtual mouse (`scripts/vpointer.c`) for
   the main flows: launcher, settings pages, Wi-Fi, window buttons.
3. **QEMU boots** every phase (live, persistent, installed BIOS + UEFI).
4. **HP checks** for what a VM can't do (GPU/Hyprland, battery, Bluetooth,
   sleep).

---

## 6. Phases

| # | Phase | Main pieces |
|---|---|---|
| 1 | **Debug sweep** | screenshot + log every app; fix what breaks; click-through tests |
| 2 | **Hexa design system** | design tokens, Inter/JetBrains Mono, hex window buttons, renames (Nexus, Orbit, Deck, Signals) |
| 3 | **Settings, essentials** | Display, Sound, Power & Battery, Updates, Storage, Mouse & Touchpad |
| 4 | **Nexus + Orbit + Deck** | new launcher, dock, quick settings |
| 5 | **Nexa Mind offline** | llama.cpp backend, model manager, OCR, voice |
| 6 | **Privacy Center** | per-app internet/permissions, firewall page |
| 7 | **Apps** | Video, Photos, Backup, Mail + Calendar, Featured page, Flatpak |
| 8 | **Settings, the rest** | Users, Language, Accessibility, Notifications & Focus, Desktop, Developer |
| 9 | **Time Rewind + signed updates** | snapshots, rollback, GitHub updates (needs the repo) |
| 10 | **Nexa Lock + Constellation** | own lock screen, window overview |

Each phase is shippable on its own; the ISO is rebuilt and tested at the
end of each.
