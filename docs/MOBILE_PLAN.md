# Nexa Mobile: NexaOS on phones (plan, 2026-10-25)

How NexaOS gets onto phones: the base, the first phones, the touch shell,
calls and messages, battery, security, apps, and phases. A plan only: the PC
system comes first, and (AGENTS.md) the installer last.

## 1. The base: Alpine on phones already exists

NexaOS is Alpine Linux underneath, and so is **postmarketOS**, the largest
project putting mainline Linux on phones: 723 device models supported as of
February 2026, its 25.12 release on Alpine 3.23
([Wikipedia](https://en.wikipedia.org/wiki/PostmarketOS),
[AlternativeTo](https://alternativeto.net/news/2025/12/postmarketos-25-12-brings-alpine-3-23-base-gnome-49-plasma-mobile-6-5-and-kernel-updates)).
Devices are graded *main* (reliable for what the phone is for: calls, SMS,
data, Wi-Fi, Bluetooth), *community* and *testing*
([device categories](https://docs.postmarketos.org/pmaports/main/packaging/device-categorization.html)).
New phones start in *testing*: the Fairphone 6 was added in 2025 with Wi-Fi,
cellular and camera not working yet
([AlternativeTo](https://alternativeto.net/news/2025/8/postmarketos-adds-fairphone-6-iphone-x-major-apple-and-vr-support)).

**Decision: mainline Linux, built on postmarketOS's device ports.** The other
road is Droidian or Ubuntu Touch, which run the phone's own Android drivers
through Halium/libhybris. It makes more hardware work today, but depends on
closed drivers and a kernel that stops getting updates when the maker drops
the phone; KDE's Plasma Mobile moved away from it
([Liliputing](https://liliputing.com/mobile-linux-distributions/),
[discussion](https://scribe.disroot.org/post/13095/45575),
[Lemmy](https://lemmy.imagisphe.re/post/1851664/4866458)). NexaOS's rules
(open, Rust/C, security first, everything checked) fit mainline; Halium can
be looked at later only for phones mainline can't reach.

**The fastest real start:** NexaOS becomes a postmarketOS *UI*
(`postmarketos-ui-nexa`): one package that pulls in the Nexa shell, apps and
services. postmarketOS builds the image for any supported phone with its
kernel, firmware and boot tooling; NexaOS supplies everything you see and use.
Later NexaOS can build its own images from the same device packages.

## 2. First phones (and how we test)

| Device | Why | Role |
|---|---|---|
| **OnePlus 6 / 6T** (Snapdragon 845, used ~$80-120) | one of the best-supported mainline Qualcomm phones: GPU (freedreno), modem, Wi-Fi, calls | the reference phone, like the HP EliteBook for PCs |
| **Google Pixel 3a** | common, cheap, well documented | second phone |
| **PinePhone / PinePhone Pro** | designed for Linux, open hardware | development board (slow, but nothing hidden) |
| **QEMU aarch64 (virt)** | no hardware | every-commit tests, like the live-boot test today |

Whether a phone is ready is decided from its postmarketOS wiki page (calls,
modem, camera, suspend), not assumed.

## 3. The touch shell (Nexa's own, not a copy of iOS or Android)

The compositor: **phoc** (wlroots, built for phones: touch, rotation, the
on-screen keyboard, screen blanking) at first; the Nexa shell parts are
layer-shell clients, as on the PC (the Dock, Nexus, the For you card and the
drives layer already are). On top:

- **Home: the honeycomb.** Apps as NexaHexa tiles on a hexagon grid (the Hive
  plan, docs/DESKTOP_PLAN.md, at phone scale), the For you row (Nexa
  Patterns) at the top, swipe up for Nexus (search, all apps, ask Nexa).
- **Status bar** (time, signal, Wi-Fi, battery) and, swiped down, **quick
  settings**: Wi-Fi, mobile data, Bluetooth, airplane, flashlight,
  brightness, rotation lock, Do not disturb. Glass, edge-lit, the theme's
  colours.
- **Gestures:** swipe up home, swipe up and hold for open apps (cards),
  swipe from the edge to go back. **Lock screen** with notifications and
  the clock; PIN or password; fingerprint where the phone's sensor has a
  mainline driver.
- **On-screen keyboard**: an existing Wayland input-method keyboard first
  (squeekboard or stevia), themed; a Nexa keyboard (Rust, with word
  suggestions from the on-device model) later.
- **Same look as the PC**: themes, glass, icon styles, NexaHexa. One design.

## 4. Phone things the PC never needed

| Need | How |
|---|---|
| Calls and SMS | ModemManager (+ eg25-manager on PinePhone); the existing GNOME Calls and Chatty at first, themed; Nexa Phone and Messages apps (Rust UI logic + thin GTK) later. **Emergency calls are tested on every release** |
| Mobile data | ModemManager + iwd/NetworkManager-free plumbing in the broker (APN settings in Settings) |
| Wake on a call | suspend with the modem allowed to wake the phone; nexa-kerneld learns phone power states |
| Battery | goal: a day of normal use. Measured per release (screen on, suspended, on a call). nexa-kerneld's power steering, Nexa Patterns warming apps you are about to open |
| Camera | libcamera; a Nexa Camera app once the reference phone's camera works mainline |
| Rotation, proximity, light | iio-sensor-proxy; the screen turns off at your ear during calls |
| Notifications | mako-like daemon from the PC, with a phone layout |

## 5. Security and privacy (same rules as the PC)

- Every root action through **nexa-brokerd**; apps confined by **nexa-run**
  (Landlock) with the same `.nexa` permissions, plus `phone`, `sms`,
  `contacts`, `location` (already in the list), `camera`.
- **Disk encryption on by default** (a phone gets lost): unlocked with the PIN
  at boot by an on-screen unlocker in the initramfs (unl0kr, from
  postmarketOS), later bound to hardware where the phone allows.
- Signed images and A/B updates (TODO I6). A phone must never be bricked by
  an update.
- Nexa Mind stays on the phone (a smaller model; nothing sent anywhere).

## 6. Apps

- The `.nexa` format already has the architecture (`ARCH`): packages get an
  `aarch64` build; the Store shows what runs on this device. nexa-pack and
  the Store's catalogue gain the architecture field in practice.
- NexaOS's own apps must already work at 360 px wide (WRITING_NEXA_APPS.md):
  each is checked in a phone-sized headless test (portrait 360x780, touch
  events) and given a one-column layout where it doesn't fit.
- Rust tools cross-compile to `aarch64-unknown-linux-musl` (the build
  container gains the target); C tools build in an aarch64 Alpine container.

## 6b. PC and phone together

**Nexa Link** (docs/NEXA_LINK.md): once paired, the PC and the phone connect
by themselves: notifications, calls and texts on the PC, files, find my
phone. Encrypted, no cloud, every feature a separate permission.

## 7. Phases

| Phase | What | Done when |
|---|---|---|
| **P0 Port** | aarch64 builds of every Rust/C tool and the Python apps; `postmarketos-ui-nexa` package; image for QEMU aarch64 | the NexaOS desktop starts in QEMU aarch64 (test-live-boot for arm) |
| **P1 Reference phone** | OnePlus 6 image: boots, Wi-Fi, touch, the shell, rotation, on-screen keyboard | screenshots from the phone; apps open and work by touch |
| **P2 Phone shell** | home (honeycomb), status bar, quick settings, gestures, lock screen, notifications | daily-use checklist on the phone |
| **P3 Calls and messages** | modem, calls, SMS, mobile data, emergency calls, wake on call | a day with a SIM: calls in and out, texts, data |
| **P4 Battery and sensors** | suspend that lasts, proximity, light, measured battery life | a full day on one charge, measured |
| **P5 Apps** | every Nexa app adapted to phone size; Store with aarch64 packages; Camera | the app checklist at 360 px |
| **P6 Release** | encrypted by default, signed A/B updates, a flashing guide (and a `nexa-flash` tool, Rust, around fastboot) | someone else installs it from the guide |

## 8. What it needs from you

- A **OnePlus 6 or 6T** (used) as the reference phone. It can be fully
  flashed back to Android if needed.
- A spare SIM for P3.
- The PC system first: the batch of fixes waiting for the next ISO build.
