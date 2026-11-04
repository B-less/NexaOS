# Nexa Identity: the plan (TODO, 2026-11-04)

Goal: NexaOS should not look like macOS, Windows, Android, GNOME or
ChromeOS anywhere a person looks every day. Boot, lock screen, Settings'
Living Field and the Living backgrounds are already our own; the Dock
(magnifies like macOS), quick settings (`control.py`, a "Control Centre"),
pop-up notifications (mako) and the "Launchpad" name are not. Names and
shapes come from docs/ROADMAP.md section 1 (Hexa).

## Ground rules

- [ ] Native code does the work: C for what is drawn on the screen edge
      (layer-shell), Rust for services; Python only as a thin layer
      (AGENTS.md).
- [ ] No bottom taskbar, no centred magnifying dock, no corner pop-up boxes,
      no accent colour taken from the wallpaper, no activity timeline.
- [ ] Everything follows the theme live, honours reduced motion and the
      2 GB profile, and works from the keyboard.
- [ ] Measured: memory and CPU of each new part, before and after, in this
      file.

## Phase 1: replace the borrowed parts

- [x] **Orbit** replaces the Dock (2026-11-04; first real use with the next ISO): a slim rail on the left edge, icons in
      hexagon frames, no magnification; a ring around a running app shows
      its real activity (CPU and network), a dashed ring when it waits on
      you; folds to a thin line of light when idle. C (GTK3 +
      gtk-layer-shell, cairo). Design: section "Orbit" below.
- [ ] **Deck** replaces quick settings: cards that slide out one after
      another from the bar's right edge (Wi-Fi, sound, VPN, Quiet, now
      playing), each opens in place.
- [ ] **Signals** replaces mako: notification capsules that dock into the
      bar's edge, stacked per app, actions inline. Own notification service
      (Rust, org.freedesktop.Notifications).
- [ ] Rename leftovers: "Launchpad" becomes Nexus everywhere; no "Control
      Centre" wording.

## Phase 2: windows our own way

- [ ] **Strata**: workspaces as layers behind each other; Super+scroll
      dives through them; each layer tints the Living background.
- [ ] **Constellation**: every window as a star grouped by app, lines
      between windows of one project (same folder, from FileTrail and open
      files). Rust builds the graph, C draws it.
- [ ] **Tethers**: windows that belong together share a thin coloured edge;
      moving one offers to move the rest.

## Phase 3: the Hive desktop

- [ ] Build D1 to D6 of docs/DESKTOP_PLAN.md.

## Phase 4: signature touches

- [ ] **One hexagon** from boot to desktop: the boot hexagon becomes the
      lock screen clock, then the Nexus button.
- [ ] **System weather**: the Living background dims slowly as the battery
      runs low, its lines break while offline, it moves faster under load
      (reduced motion and the 4.5:1 contrast rule always win).
- [ ] **Nexa sounds**: our own sound set from a small C synthesiser
      (start-up chord, a tone per kind of Signal).
- [ ] **Quiet modes** (Work / Study / Night), shared with the Magic Layer's
      Quiet: background, which Signals get through, power profile.

## Orbit

`tools-c/nexa-orbit` (C: GTK3 + gtk-layer-shell, cairo, Pango), its own
process, started by both sessions and kept running by Nexa Doctor. It
replaces the Python Dock (`shell/nexa_shell/dock.py`, removed).

**Shape.** A vertical rail on the left edge, centred, 8 px from the edge.
Its ends are pointed, like a hexagon stretched tall, so it reads as one
long Hexa cell, not a tray. At the top sits the NexaOS mark (opens Nexus),
then the pinned apps, a short divider, then running apps that aren't
pinned. Every icon sits in a pointy-top hexagon frame. Nothing magnifies;
the name appears in a capsule to the right of the rail.

**State, drawn on the hexagon frame:**

| State | Look |
|---|---|
| running | the frame outlined in the accent |
| busy | a brighter stroke traced around the frame, its length the app's CPU use (all its processes, 0 to 100 % of one core, eased) |
| in front | the frame filled with the accent, faintly |
| waiting on you (urgent) | dashed frame in the warning colour, slowly turning |
| starting | a short stroke circling the frame until its window appears (8 s at most) |
| minimised | the outline at half strength |

Reduced motion (GTK's animation setting, set by Settings > Accessibility):
nothing circles or turns; dashes stay still; folding is instant.

**Use.** Click: open, or bring the app forward (out of minimised). Middle
click: a new window. Right click: Open / New window, Pin to Orbit or
Unpin, Close all windows. Super+O gives Orbit the keyboard: Up/Down
(Home/End), Enter opens, the Menu key or Shift+F10 opens the item's menu,
Escape gives the keyboard back.

**Fold.** `fold=yes` in `~/.config/nexa/orbit.conf`: the rail keeps no
space for itself and folds into a thin line of light on the left edge,
unfolding when the pointer touches it, folding 0.7 s after it leaves.
Default `fold=no` (the rail keeps its strip, windows open beside it).

**Data, all event-driven.** Windows: Sway IPC `window` events / Hyprland's
`socket2` (urgent from its `urgent>>` event), re-read after a 100 ms
settle; polling every 2 s only if the event socket fails. Activity:
`/proc/*/stat` every 2 s, only while apps run and the rail is open; a
window's process and its children count for the app. Network per app is
not measured yet (needs per-app accounting; Phase 4b's per-app internet
control will give it). Commands to the compositor only accept app ids of
`[A-Za-z0-9._-]` into its files, and name windows in commands by their
numeric id (Sway `con_id`, Hyprland address), so no command can be
smuggled in a window's app id.

**Files.** Pinned apps and `fold`: `~/.config/nexa/orbit.conf`, written
atomically. Colours: `~/.config/nexa/lock.conf` (nexa-theme, plus
`opacity`), watched for live theme changes; icon theme changes are
followed too. Apps it starts are watched and abnormal ends written to
`~/.local/state/nexa/exits.jsonl` for Why, as the Dock did.

- [x] Engine pieces with unit tests (config, /proc parsing, CPU maths,
      Exec field codes, app id check): 28 checks in `make test-orbit`
- [x] Rail, states, menus, keyboard, fold; theme and icon-theme live
- [x] Both session configs, Super+O, Nexa Doctor part; Dock removed
- [x] Headless Sway test (`make test-orbit`, 26 checks, screenshots in
      dist/orbit-test): opens an app with a real click, "starting" then
      running, the window opens beside the rail, a busy app measured over
      50 % and an idle one under 20 %, clicks bring apps forward, urgent
      marked, an unknown app's window shown after the divider and gone when
      it closes, the name on hover, the menu, orbit.conf and a theme change
      followed live, Super+O on and off, fold and unfold
- [ ] Measured: 15.3 MB PSS / 32.4 MB RSS in the headless test, where no
      other GTK program shares its libraries (the Python Dock alone was
      28.1 MB PSS, docs/MICROBRAIN.md). Still to measure in the 4 GB VM:
      nexa-parts without the Dock plus nexa-orbit, against nexa-parts before
- [ ] Not yet tested: Hyprland (written to its IPC, as nexa-living's), a
      real mouse and HiDPI screens; the GTK menu's look in the real theme
- [ ] Later: drag to reorder, "Pin to Orbit" in Nexus, a Settings page
      (pinned apps, fold), network activity per app
