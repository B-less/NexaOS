# Nexa Magic Layer + Living Visual Engine: the plan (TODO)

Started 2026-10-25 from the "NexaOS — Magic Layer & Living Visual System"
brief, filtered to fit NexaOS as it is. A box is ticked only when the thing
works and its test passes; "unverified" means built but not yet checked
where it matters (real hardware, the ISO).

**In one line:** one small C renderer gives first-party apps living
backgrounds behind their glass; MicroBrain (already running, Rust) hosts the
18 Magic Layer features as modules; nothing new runs as root, and every
feature has a real switch in Settings > Magic Layer.

## 1. What the brief asked for, filtered

| The brief | NexaOS reality | So we do |
|---|---|---|
| an animated background inside every app | apps are Python/GTK3; a 30 fps cairo loop in each Python app would cost a core on a 2 GB laptop | **one** renderer, `nexa-living` (C, layer-shell, background layer): the wallpaper plus, inside each first-party window's area, that app's style. Apps already have glass (translucent windows); an app with a living background turns its glass on. One process, one frame clock, paused where nothing shows |
| shaders, GPU | Sway runs pixman (CPU) in VMs; Hyprland on real GPUs | 2D procedural drawing with cached geometry; frame caps 60/30/12/static; damage only where it moves |
| mouse reacting, click ripples | the background layer only gets the pointer over the bare desktop | pointer effects on the desktop; apps send their own clicks/focus to the renderer (one tiny socket message) |
| apply to third-party apps | only via theming | third-party windows get nothing inside; said in Settings |
| System Monitor, Terminal apps | Nexa Pulse is the monitor; foot is the terminal (has its own alpha) | Pulse gets the Energy Pulse style driven by real load from MicroBrain; foot gets Digital Rain behind its own alpha |
| 18 features as services | MicroBrain already runs per user, event-driven, with a scheduler, budget and Landlock | the features are MicroBrain modules (Rust) + thin GTK screens; privileged bits via nexa-brokerd |
| Rewind for all settings | Interface DNA keeps undo; themes/topbar/wallpaper are files | one Rust journal of settings changes with the previous file content; undo only what has a stored previous state |
| Lifeboat | nexa-sessiond owns VTs now | a Rust text-mode recovery screen on its own VT, reached with a key even if the desktop hangs |
| GestureKeys | compositors don't expose raw pointer streams to clients | holding the gesture key (a compositor binding) opens a transparent overlay (C) that reads the drag and recognises the shape |
| Gravity (linked windows) | Sway IPC / hyprctl can move windows and report moves | a module that follows window events and moves the partner; floating windows only (tiled ones already cooperate) |

Not doing: a browser engine or one animation process per app; restyling
third-party apps' insides; recording screen content; a resident AI model.

## 2. Ground rules

- [ ] Every switch in Settings > Magic Layer changes real behaviour.
- [ ] Living backgrounds never sit above content, never take input, never drop text contrast below 4.5:1 (the app's glass alpha is raised if needed).
- [ ] Frame caps: High 60, Balanced 30, Low 12, Static 0; hidden/minimised windows' regions don't animate; pressure (MicroBrain mode), battery saver, reduced motion and the 2 GB profile lower quality with hysteresis.
- [ ] Measured, not claimed: CPU %, RSS, frame time per quality, in docs/MAGIC_LAYER.md.
- [ ] Learning features opt-in; logs and journals bounded; no clipboard history by default.
- [ ] No shell commands from gestures/Orbit: typed actions only (nexa_common.brain.run).

## 3. Phases and features

### Phase 1: inspection
- [x] Toolkit (GTK3/Python, C/Rust), compositors, glass, MicroBrain, broker, sessiond, Interface DNA known.

### Phase 2: shared core
- [x] `~/.config/nexa/magic.json`: every feature's switch and preferences, versioned; read by nexa-living and the apps (`nexa_common.magic`, test_magic.py). MicroBrain reads it as each feature lands.
- [ ] MicroBrain `magic` request family: status of every feature, set preferences.

### Phase 3: Living Visual Engine (`nexa-living`, C)
- [x] Renderer: layer-shell background surface per output, frame clock, quality caps, damage tracking (pixman regions), static fallback.
- [x] The ten styles: Cyber Circuit, Quantum Flow, Neural Network, Holographic Grid, Digital Rain, Cosmic Nebula, Energy Pulse, Orbital Field, Living Geometry, Nexa Signature.
- [x] Window regions from the compositor (Sway IPC events + a 4 Hz look while animating; Hyprland sockets, unverified): per-app style inside each first-party window, brighter when focused, still when not, nothing for hidden windows.
- [x] App events: open (fade-in), click (ripple), level (real activity). Every Nexa app sends its clicks (title bar code; tested with a real pointer, `nexa-vpointer`); Pulse (CPU load), Music (playing), Files (copy progress), Assistant (thinking) send levels.
- [x] Real data: Energy Pulse in Pulse follows the CPU load Pulse measures.
- [x] Policy: MicroBrain mode / battery / reduced motion (also Interface DNA motion none) / 2 GB → quality; measured frame time steps down after 2 s slow, up after 10 s fast.
- [ ] Apps: Settings first, then Files, Pulse, Store, Assistant, Music, others; foot via its alpha.
- [x] Test: `make test-living` (15 checks + measurements), `make test-magic-page` (16).

### Phase 4: core Magic Layer
- [ ] Peek (preview without opening the app; no document code run)
- [ ] Orbit (radial actions by context; keyboard equivalent)
- [ ] Rewind (timeline of reversible settings changes)
- [ ] FileTrail (where did I move it: nexa-fileops journal)
- [x] Why (slow / crash / memory / network / stopped task, from real evidence, proven vs possible): why.rs in MicroBrain, exit records from the desktop, Pulse's Why tab, Nexus answers, Settings switch; `make test-why`.

### Phase 5: workspace and interaction
- [ ] Gravity · [ ] Hold · [ ] Thread · [ ] Tomorrow · [ ] GestureKeys

### Phase 6: reliability
- [ ] Passport (known drives by filesystem UUID) · [ ] Postbox (queued transfers, resume only where the server supports ranges) · [ ] Ready (before shutdown) · [ ] Lifeboat (recovery VT)

### Phase 7: convenience
- [ ] Appetite (measured app RAM/CPU history) · [ ] Quiet (focus profiles) · [ ] Clipboard Sense (opt-in) · [ ] Surprise (tips, never repeated once dismissed)

### Phase 8: Settings > Magic Layer
- [x] Living Background Studio (desktop style, quality, speed, brightness, density, reaction, dimming, window opacity, fps cap, battery, reduced motion, preview, measured numbers, defaults). Accent: the theme's (a theme change recolours it).
- [x] Per-app styles, apply to several ticked apps, off per app
- [ ] A page per feature: switch, explanation, preferences, shortcut, privacy, measured cost, reset

### Phase 9-10: optimise, measure, test, document
- [ ] docs/MAGIC_LAYER.md with measurements; checklist of done / partial / unverified / blocked.
