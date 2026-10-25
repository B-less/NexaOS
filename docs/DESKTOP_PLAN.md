# The Hive: app icons and files on the NexaOS desktop (plan, 2026-10-25)

Today the desktop shows the wallpaper, the For you card (top right), USB
drives and SD cards (top left, `nexa_shell drives`) and the Dock. This is the
plan for putting **apps, files, folders and drives** on it in NexaOS's own
way. It is not a grid of squares copied from another system: everything sits
in a **honeycomb**, the pointy-top hexagon that is the NexaOS mark.

## 1. The idea

- **One honeycomb.** The desktop is a lattice of pointy-top hexagon cells
  (about 96 px across, scaled with the screen). Every item fills one cell:
  an app, a file, a folder, a drive. Cells interlock, so a full desktop
  looks like one surface, not rows of loose icons.
- **What is on it.** Files and folders are the real contents of
  `~/Desktop` (nothing is copied or hidden). Apps are links you pin there
  (`~/Desktop/*.desktop`, from Launchpad or the Dock: "Pin to desktop").
  Drives appear on their own while plugged in.
- **Clusters, not clutter.** Items of one kind can gather into a
  **cluster**: a hexagon flower of up to seven cells (one in the middle, six
  around it) - "Pictures", "Documents", "Apps", "Today". A cluster folds
  into one cell with a count, and opens in place when clicked. The desktop
  offers to cluster when it gets crowded; nothing moves without you.
- **Zones.** The top-left corner is for drives, the top-right for the For
  you card, a strip above the Dock stays clear. Pinned apps start from the
  left, new files from the right, so the two never mix by accident.
- **Every kind of file looks like itself.** Pictures, videos and PDFs show
  a preview inside the hexagon (nexa-thumb, the same C thumbnailer Files
  uses); folders show the pictures inside; apps show their NexaHexa tile;
  other files show their type icon. A new file glows softly for a minute
  ("Today" cluster: everything you saved or downloaded today).

## 2. Using it

| Action | What happens |
|---|---|
| Click / Enter | open (a drive: open in Files; a cluster: unfold it) |
| Drag | the item moves cell to cell, snapping; the others make room |
| Drag onto a cluster or folder | into it; onto a drive: copy to it; onto the Trash in the Dock: Trash |
| Drop a file from Files | moved to `~/Desktop` (Ctrl: copied) |
| Right-click an item | Open, Open with, Rename, Move to Trash, Pin/Unpin, Add to cluster, Eject (drives) |
| Right-click the desktop | New folder, Paste, Arrange (by kind / name / date), Clean up (cluster everything), Show desktop items on/off |
| Rubber-band / Ctrl / Shift | select several; act on all of them |
| Keyboard | arrows move between cells (six directions on a hex grid: also Home/End and PgUp/PgDn for the diagonals), Space to select, F2 rename, Delete to Trash |

Positions are yours: what you put somewhere stays there across restarts and
screen changes (stored as hex coordinates, not pixels, so a resolution
change keeps the layout).

## 3. How it is built (NexaOS rules: Rust for the work, Python only as the thin layer)

- **`nexa-desktopd` (Rust)**: the layout engine. Hex-grid maths (axial
  coordinates, neighbours, distance, the spiral that finds the nearest free
  cell), placing new items in their zone, keeping each item's cell
  (`~/.config/nexa/desktop.json`), clusters, "make room" when something is
  dropped on an occupied cell, a new arrangement when the screen changes.
  It watches `~/Desktop` (inotify) and `/run/nexa/drives.json`, and prints
  the layout as JSON lines for the layer to draw. Landlock: reads home, writes
  only its own file.
- **The desktop layer (Python, GTK, layer-shell BOTTOM)**: draws the cells
  and handles input; asks the engine for every change. File actions go
  through nexa-fileops (Rust) like Files (Trash, undo, never overwrite);
  thumbnails through nexa-thumb (C); drive actions through the broker.
- **Themes**: cells, the selection and the glow use the theme's colours and
  follow a theme change live (the drives layer already does).
- **Performance**: the engine does the work; the layer redraws only what
  changed. Target: a desktop of 200 items opens in under 300 ms on the
  HP EliteBook, idle CPU zero.

## 4. Phases

| Phase | What | Done when |
|---|---|---|
| D1 | Engine: hex grid, zones, positions, watching ~/Desktop; the layer shows files and folders in cells, open, drag to move | headless test: files appear, move, survive a restart and a resolution change |
| D2 | Apps on the desktop: "Pin to desktop" in Launchpad and the Dock; NexaHexa tiles | pin, unpin, launch in the headless test |
| D3 | Thumbnails, new-file glow, the "Today" cluster | screenshots |
| D4 | Clusters: make, add, fold/unfold, Clean up | test: 30 files become clusters, nothing lost |
| D5 | Drag between desktop, Files, drives and the Trash; selection; keyboard | the click tests with the virtual mouse (common/tests/vpointer.c) |
| D6 | Settings > Desktop: show items, cell size, which zones | |

Already done toward it: drives on the desktop (top-left zone, 2026-10-25).
