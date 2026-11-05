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
- [x] **Deck** replaces quick settings (2026-11-04; first real use with the next ISO): cards that slide out one after
      another from the bar's right edge (Wi-Fi, sound, VPN, Quiet, now
      playing), each opens in place. Design: section "Deck" below.
- [x] **Signals** replaces mako (2026-11-05; first real use with the next ISO): notification capsules that dock into the
      bar's edge, stacked per app, actions inline. Own notification service
      (Rust, org.freedesktop.Notifications). Design: section "Signals" below.
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

## Phase 5: ideas no other desktop has (planned 2026-11-05)

Each builds on a part NexaOS already has. Same rules as above: C or Rust,
follows the theme live, honours reduced motion and the 2 GB profile, works
from the keyboard, measured. Suggested order: Layer moods, Edge glow, Chalk;
then Lantern and the pointer set; then Glyphs and Hex Ring; Then after
Backup.

- [ ] **Layer moods** (Strata): each layer has its own mood: which Signals
      get through, a Quiet mode, a Living style, a power profile. Diving
      to layer 3 ("Work") silences chat; layer 5 ("Night") dims. The place
      sets the mood, not the clock. Build: `strata.conf` per-layer keys,
      nexa-signald learns the current layer, a Settings page. Small.
- [ ] **Edge glow** (Signals + Living): a window that finished something or
      needs you glows softly at its own edge, in the accent, instead of a
      box in a corner; it fades once the window is looked at. Living
      already knows every window's place and draws it on the background,
      just outside the window. Small to medium.
- [ ] **Glyphs**: Super+; labels every button, link and field on the
      screen with two letters; typing them clicks it. Every app, through
      AT-SPI. C overlay (layer-shell, keyboard grab) + at-spi2-core.
      Medium to large.
- [ ] **Chalk**: a short note written on a layer's background ("call the
      bank"), behind the windows, in the theme's colours; it stays with
      that layer. Living draws it; a tiny C editor;
      `~/.config/nexa/chalk.json`. Small.
- [ ] **Hex Ring**: Super+V opens the last 8 copied things (text, pictures,
      files) as a ring around the pointer; point and release pastes. Off
      until switched on, nothing from password apps
      (`x-kde-passwordManagerHint`), never leaves the PC. C,
      wlr-data-control. Medium.
- [ ] **Lantern**: Super+L dims every other window and slows the
      background; an optional timer shows as a hexagon slowly closing on
      the window's edge. Window opacity through Sway/Hyprland IPC, the
      timer in Living. Small.
- [ ] **Then** (after Backup): a slider in Files shows a folder as it was
      yesterday or last week, from Backup's snapshots; drag a file out to
      get it back. Medium.
- [ ] **Hexa pointer set**: NexaOS's own cursor theme, hexagon-based; busy
      is a slowly turning hexagon. SVG cursors made by a script. Small.

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

## Deck

`tools-c/nexa-deck` (C: GTK3 + gtk-layer-shell, GIO D-Bus). It replaces
the Python Control Centre (`shell/nexa_shell/control.py`, removed) and keeps
everything it did: Wi-Fi, Do Not Disturb, volume, brightness, battery, the
notification list with Clear all, Settings, Lock, Power.

**Shape.** A column of cards on the right, under the bar. Every card is
half a hexagon: straight against the screen edge, chamfered on the side
facing the screen (the same cut as Orbit's pointed ends), not a grid of
rounded tiles. Opening, the cards slide in from the edge one after
another (40 ms apart); closing, they slide back faster. A card whose
thing is on has its chamfered edge lit in the accent. Reduced motion: no
sliding.

**Cards, top to bottom** (a card only appears when its thing exists):

| Card | Compact | Opened in place |
|---|---|---|
| Now | time, date, battery (and time left when the battery says) | Settings, Lock, Power |
| Wi-Fi | switch, connected network | nearby networks by signal; a known one connects here, a new one opens Settings > Wi-Fi |
| Bluetooth | switch, connected devices | paired devices, connect / disconnect |
| Sound | volume slider, mute | link to Settings > Sound |
| Brightness | slider (laptops) | - |
| VPN | switch for the tunnel in use (or the first) | every imported tunnel |
| Quiet | switch: notifications wait (mako's do-not-disturb, until the Quiet modes) | - |
| Playing | title, artist, back / play-pause / next (any MPRIS player) | - |
| Signals | count and the newest | up to 20, Clear all |

**Use.** `nexa-deck` toggles it (Super+N, the bar's buttons); `nexa-deck
wifi` (bluetooth, sound, vpn, signals, ...) opens with that card opened,
so the bar's Wi-Fi icon opens Deck on Wi-Fi. A click outside the cards or
Escape closes it; Tab moves through every control. One instance
(GtkApplication): running it again closes it.

**Data.** Read when it opens and every 2 s while open, all asynchronous
(nothing blocks the window): iwd and BlueZ over the system bus, MPRIS over
the session bus, `wpctl`, `makoctl`, `/sys/class/power_supply` and
`/sys/class/backlight`. Changes: iwd / BlueZ properties and methods, wpctl,
makoctl; brightness and VPN through the broker (`set_brightness`,
`vpn_up`, `vpn_down`: no password; tunnel names checked before they are
sent). The process ends when Deck closes: nothing runs while it's away.

- [x] Engine pieces with unit tests (wpctl, makoctl history, battery and
      time left, backlight, broker request and reply, tunnel names,
      signal strength, UTF-8-safe cuts): `make test-deck`. The shared JSON
      reader (tools-c/nexa-living/json.c) now turns \u escapes into UTF-8
      instead of '?'
- [x] Cards, opening in place, staggered slide, keyboard, theme
- [x] Bar buttons and Super+N open Deck (the bar's Wi-Fi, sound,
      brightness, battery and bell open it on their card; right click on
      Wi-Fi still opens the Network app); Control Centre and its Python
      parsers removed; `nexa-control` kept as a name for Deck
- [x] Headless test (`make test-deck`: 37 unit checks, 28 in a headless
      Sway with a real pointer, screenshots in dist/deck-test) against fake
      iwd, BlueZ, MPRIS, broker, wpctl and makoctl: every card shows with
      the right reading, Wi-Fi opens in place and a saved network connects,
      the Bluetooth, VPN (vpn_up through the broker) and Quiet switches,
      both sliders, Play, Clear all; `nexa-deck signals` opens that card in
      the running Deck and scrolls it into view; running it again or a
      click outside closes it and the process ends
- [ ] Measured: 26.9 MB PSS while open in the headless test (nothing else
      shares its libraries there); nothing while closed (the process ends)
- [ ] Not yet tested: Hyprland, real iwd / BlueZ / mako / PipeWire, a
      touchpad, the keyboard path (Tab, Escape: no virtual keyboard in the
      test yet)
- [ ] Later: Bluetooth pairing and Wi-Fi passwords inside Deck (today they
      open Settings), sound output devices, Quiet modes, Signals (Phase 1)
      taking over from mako

## Signals

Two programs, so the part that reads what any app sends is Rust and the
part on the screen edge is C:

- **`nexa-signald`** (Rust, `tools/nexa-signald`, the `dbus` crate over
  libdbus): owns `org.freedesktop.Notifications` on the session bus
  (Notify, CloseNotification, GetCapabilities, GetServerInformation;
  NotificationClosed and ActionInvoked signals). Cleans what it is sent
  (markup tags removed, entities decoded, control characters dropped,
  app 64 / summary 200 / body 600 characters, at most 4 actions), keeps
  the last 50 in memory only (never on disk), and decides what shows:
  Quiet holds pop-ups back (they still reach the list), muted apps
  (Settings > Notifications) never pop up, critical ones always do and
  stay until dismissed; the timeout is the sender's, or Settings' (0 =
  until dismissed). A pointer on a Signal holds its timer.
- **`nexa-signals`** (C, `tools-c/nexa-signals`, GTK3 + gtk-layer-shell):
  draws them. Each Signal is a tab hanging from the bar's bottom edge on
  the right: flat where it meets the bar, chamfered at the bottom like
  Deck's cards, sliding down out of the bar and back up into it. Not a box
  floating in a corner. One tab per app: the newest, with "+2" when that app
  has more (a click on the count shows them all); at most 4 apps, then a
  "+N more" tab that opens Deck on Signals. A thin line along the bottom
  shrinks as the timeout runs. Click: the default action (or just
  dismiss); the actions as pills; × dismisses. Critical: the warning
  colour on the chamfer.
- **`nexa-signalctl`** (Rust, the same crate): `list`, `history`,
  `dismiss --all | ID`, `mode [-a|-r|-t do-not-disturb]`, `reload`, with
  the output Deck and the bar already read from makoctl, so they switch
  with one word.

They talk over `$XDG_RUNTIME_DIR/nexa-signals.sock` (0600, peer uid
checked, JSON lines, 64 KB line limit): `subscribe` streams the whole
visible state on every change; `hold`, `invoke`, `dismiss`, `dismiss_all`,
`list`, `history`, `mode`, `set_quiet`, `reload`.

- [x] Store with unit tests: ids and replaces_id, cleaning, timeouts and
      holds, Quiet, muted apps, critical, history cap, closed reasons
- [x] D-Bus service and control socket; nexa-signalctl. The D-Bus loop
      sleeps in its own poll on the bus socket and a wake pipe, so signals
      sent from the socket threads go out at once (0 CPU ticks idle).
      libdbus is built in (the `dbus` crate's vendored feature): Alpine has
      no static libdbus and every tool links statically. The native build
      now also installs src/bin/*.rs binaries (nexa-signalctl)
- [x] Tabs on the bar's edge: per-app stacking, actions, timeout line,
      hover hold, slide, theme; Settings' position choice honoured (the
      bottom ones the other way up)
- [x] Sessions run nexa-signald and nexa-signals instead of mako; Deck,
      the bar's bell, Settings > Notifications, Nexa Doctor and the theme
      engine switched (mako's config no longer written; the package stays
      installed, unused)
- [x] Headless test (`make test-signals`: 7 unit checks, 24 end to end with
      real D-Bus calls, the real service and a real pointer, screenshots in
      dist/signals-test): tab at the top right, one per app with a count,
      an action reaches the sender (ActionInvoked, then closed), the count
      shows all, x and clicks close with the right reason, a click sends
      the default action, a pointer holds the timer, Quiet, critical in
      Quiet, dismiss --all, four apps then "+2 more", muted apps, bottom-left,
      history for Deck, the tabs reconnect when the service restarts
- [ ] Measured: nexa-signald 1.3 to 1.4 MB PSS, nexa-signals 13 to 22 MB (three headless runs; nothing else shares its libraries there)
- [ ] Not yet tested: Hyprland, real apps' notifications (Firefox-style
      images, sounds), a real bar above the tabs
- [ ] Later: keyboard access to the tabs, inline reply, images in the body,
      Quiet modes

## Strata

Workspaces 1 to 5 become five layers, one behind the other. Layer 1 is the
surface; the others lie deeper.

**Moving.** Super+scroll dives through the layers: down goes one layer
deeper, up comes back. Super+Page Down and Super+Page Up do the same from the
keyboard. With Shift held, the window in front comes along. Moves stop at
layers 1 and 5, with no wrapping. `tools-c/nexa-strata` is a small C command
(no GTK, starts in about a millisecond) that both sessions bind:
`nexa-strata dive +1|-1`, `nexa-strata carry +1|-1`, `nexa-strata current`.
It asks the compositor for the current layer and moves to the next one by
number, so it can reach an empty layer. A touchpad sends a stream of scroll
events, so moves less than 150 ms apart are ignored (a timestamp in
`$XDG_RUNTIME_DIR`). Super+1 to 5 still jump straight to a layer.

**The stack.** Whenever the layer on screen changes, whatever caused it
(scroll, keys, the bar, an app asking for attention), a small stack of five
tilted glass sheets shows near the bottom of the screen for about a second.
Each sheet is a layer: deeper ones sit higher, smaller and fainter. Layers
you dived past fall towards you and fade out. The sheet you are on is
solid, with its windows' app icons on it and its number in a hexagon. A
column of five hexagons beside the stack shows where you are: filled for
the current layer, outlined for layers with windows, faint for empty ones.
The sheets glide to the new depth in 220 ms. With reduced motion there is
no glide and no fade. The stack takes no input and never covers the
pointer's clicks.

It runs inside Orbit's process, so it shares Orbit's compositor
connection, app icons and theme. That costs about 1 MB, where a second GTK
program would cost about 13 MB. Its window exists only while it is shown.

**Tint.** Every layer has its own colour: the theme's accent turned 40°
around the colour wheel per layer (layer 1 is the accent itself), never a
colour taken from the wallpaper. Living lays that colour faintly over the
wallpaper of each screen (strength 0.14) and cross-fades it in 0.4 s when
the layer changes; with reduced motion the change is instant. The windows
and their styles are never tinted.

**Files.** `~/.config/nexa/strata.conf`: `show=yes|no` (the stack) and
`tint=0` to `0.3` (0 turns the tint off), watched live by Orbit and Living.
Colours come from `~/.config/nexa/lock.conf`.

- [x] Engine pieces with unit tests (layer maths, tint colours, config,
      sheet geometry): 25 checks
- [x] `nexa-strata` command; layer per window and the current layer in
      Orbit's compositor code (Sway and Hyprland)
- [x] The stack in Orbit; Living's tint
- [x] Both session configs (Super+scroll, Super+Page Up/Down, with Shift)
- [x] Headless Sway test with screenshots (`make test-strata`)
- [x] Measured: `nexa-strata` 33 KB, about 3.5 ms a call; the stack adds
      about 1 MB to Orbit while shown (26 MB to 27 MB); Living stays at 9 MB
- [ ] Later: tested in a Hyprland session (its bindings and gesture are
      written but only Sway runs in the test)
