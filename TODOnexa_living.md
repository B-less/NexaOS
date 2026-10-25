# Nexa Living Interface: the plan (TODO)

Started 2026-10-25 from the "NexaOS ∞ The Living Interface" brief,
filtered to fit NexaOS as it is. A box is ticked only when the thing works
and its test passes.

**In one line:** the computer shown as a field of real system objects
(Performance, Storage, Network, Nexa Link, Appearance, Privacy...) that
you select, open in place, and follow to what they're connected to,
instead of only rectangles and menus. Built from ordinary GTK widgets on
NexaOS's own hexagon language; no 3D, no blur, no constant animation.

## 1. What the brief asked for, filtered

| The brief | NexaOS reality | So we do |
|---|---|---|
| "a new interaction language", not a reskin | NexaOS already has a visual identity: hexagons (Nexus honeycomb, NexaHexa icons, lock screen, Dock), themes, glass | the Living Field grows out of the hexagon: system objects are hexagon cells in stable clusters, joined by thin threads |
| replace the desktop stack only with strong reason | Sway/Hyprland + GTK3 + layer-shell work and are tested | keep them; the field is GTK widgets inside the existing apps |
| a component library (NexaCore, NexaObject, NexaMorph...) | Python/GTK3 for UI (AGENTS.md), Rust for logic | components in `common/nexa_common/living/` (thin GTK), data and validation in Rust where it's real logic (Interface DNA validation, Time Fabric) |
| eight layouts for every app (Constellation, River, Origami, Matrix, Portal, Habitat, Thread, Focus) | most first-party apps are simple single-window tools | each app gets only the layouts that make sense for it (Files: Matrix grid, River list, Focus; Settings: Constellation field, Matrix list); no layout forced to fill a count |
| restyle third-party apps | only what GTK theming exposes | third-party apps get the theme, nothing more; said plainly in the Studio |
| performance and side-effect estimates | MicroBrain measures memory, CPU, budgets | the Consequence Lens shows measured numbers when we have them, "not measured" otherwise |
| Time Fabric restores anything | only what has a real backup: theme/settings files, broker config backups, apk transactions, workspaces | restore only those; everything else is shown as history only |
| Intent Surface with an AI | ZeroSearch (MicroBrain) already does deterministic intents | the Intent Surface is ZeroSearch with goals ("save battery", "less distracting") mapped to sets of real actions, shown before they run |

Not doing: a 3D engine, particles, real-time blur, a resident model,
reorganising objects because a prediction changed, hiding controls behind
the spatial view (the list view always exists and does the same things).

## 2. Ground rules

- [ ] Every object shows real data and does real things; nothing decorative pretends.
- [ ] Positions are stable: the same object is always in the same place (pins add a cluster, they don't shuffle the rest).
- [ ] Full keyboard use: arrows move between objects by position, Enter opens, Escape goes back, typing searches. Visible focus. Screen readers get names and states (ATK via GTK).
- [ ] The list view (Settings' sidebar) reaches every function the field does.
- [ ] Reduced motion and the 2 GB profile: no transitions at all.
- [ ] Redraw only on change; refresh live numbers only while the field is on screen.
- [ ] Privileged changes only through the broker, as now.
- [ ] Shapes as well as colours for state (● fine, ▲ needs attention, ■ off), never colour alone.

## 3. Phases

### Phase 1: audit (done during the MicroBrain work)
- [x] Desktop stack, toolkit, Settings structure, Nexus, Files, broker, MicroBrain known; baseline measured (docs/MICROBRAIN.md).

### Phase 2: the visual and interaction grammar
- [x] docs/LIVING_INTERFACE.md: the field, clusters, objects, threads, the three depths (compact → open in place → full page), keyboard model, state shapes, motion rules.

### Phase 3: the Living Field in Settings (first working version)
- [x] Settings opens on the field ("Overview"); the sidebar stays as the list view.
- [x] 15 objects with live summaries and state shapes (from MicroBrain, Nexa Link, Bluetooth, Wi-Fi, Patterns, Nexa Doctor, /sys), refreshed every 5 s off the UI thread, only while visible.
- [x] Open in place: real controls (resource mode, "why is it slow", themes, glass, Bluetooth, Nexa Drop, Patterns and habit learning, workspaces save/restore, MicroBrain features, battery health, lock, Trash); "All settings"/"Open the app" for the full page.
- [x] Threads under the cells; selecting an object lights its partners.
- [x] Search dims non-matches, Enter opens or runs a ZeroSearch command; pin adds a Pinned cluster without moving others; Escape steps back.
- [x] `make test-living-field`: 15 checks with the real services (opens, shows, mode change reaches MicroBrain, arrows by position, search, pin order).
- [ ] Measured: Settings' memory before and after.

### Phase 4: components
- [ ] `nexa_common/living`: NexaCore, NexaObject, NexaCluster, NexaMorph, NexaThread, NexaLens, NexaCommand, NexaFocus as reusable GTK pieces.

### Phase 5: functional Settings everywhere
- [ ] Every object's controls wired to real services; Display and Sound objects (currently no pages: wlr-randr / wpctl).

### Phase 6: Interface DNA and App Interface Studio
- [ ] A versioned DNA schema (layout, navigation, density, geometry, appearance, accent, motion, accessibility), validated in Rust against what each app declares it supports; presets, per-app overrides, import/export, migration, reset.
- [ ] Studio in Settings: pick an app, see what it supports, preview, apply, undo, defaults.

### Phase 7: first-party apps
- [ ] Files: Matrix (grid), River (list by time), Focus (one folder, no chrome).
- [ ] Others where a second layout is genuinely useful.

### Phase 8: Time Fabric and Consequence Lens
- [ ] History of theme, layout, workspace, settings and package changes (bounded, searchable), restore where a real backup exists.
- [ ] Before a meaningful change: what changes, what it affects, measured cost if known, permission needed, whether it can be undone.

### Phase 9: Intent Surface
- [ ] Goals ("save battery", "make the desktop less distracting", "make controls larger", "prepare my coding workspace") → a shown list of real actions → approve → done; built on ZeroSearch.

### Phase 10: optimise and validate
- [ ] Accessibility pass (keyboard, screen reader names, high contrast, text size), memory/CPU measurements, the 2 GB profile.
