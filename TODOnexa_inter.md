# Nexa MicroBrain: the plan (TODO)

Started 2026-10-25 from the "Master development prompt" brief, filtered to
fit NexaOS as it actually is. Each feature lists what already exists, what
to build, and how we'll know it works. A box is ticked only when the
feature runs and its test passes, not when its files exist.

**In one line:** one small Rust service, `nexa-microbrain`, that learns,
predicts, automates and protects, with no large AI model. It must fit in
about 100-250 MB extra on a 4 GB PC (80-180 MB on 2 GB), stay under ~2 %
CPU when idle, and never be the reason an old computer feels slow.

Also in this batch: **Nexa Defender** (protection against threats to the
system and the PC), a **better Files app**, and our own ideas (section 7).

---

## 1. What the brief got wrong for NexaOS (filtered out or changed)

| The brief says | NexaOS reality | So we do |
|---|---|---|
| systemd, `nexa-microbrain.service` | OpenRC; desktop parts start with the session (Sway/Hyprland `exec`) and Nexa Doctor restarts them | a per-user session service started like `nexa-patternd`, supervised by Nexa Doctor; root work only through `nexa-brokerd` (OpenRC) |
| build 12 features from scratch | half of it exists: `nexa-patternd` (habits, predictions, preloading), `nexa-kerneld` (PSI pressure, OOM kills, CPU floor for the app in front), `nexa-doctor` (health, fixes, restarts), `nexa-tools` (duplicates, tidy, Trash, undo), `nexa-fileops` (Files' engine), the broker with an audit log, Nexus search | MicroBrain is the shared brain *on top* of these; it reuses their data and code, and absorbs a service only where that saves a process |
| SQLite for memory | every service today keeps JSON/JSONL in its own folder | SQLite (`rusqlite`, bundled) only for MicroBrain's own store, if measurements say it's worth ~1 MB of binary; schema migrations either way |
| examples with VS Code | not in the image | examples use NexaOS apps (Editor, Terminal, Files) and any app with a .desktop file |
| Rust async runtime | the other services use threads and blocking I/O and are small | threads + bounded channels; an async runtime only if a measurement shows a need |
| "cgroups v2 freezer" | Alpine 3.24, cgroups v2 mounted; `nexa-kerneld` already makes cgroups | SmartFreeze uses the same cgroup tree (user-owned scope), never touches system cgroups |
| "optional local AI behind a flag" | Nexa Mind exists (llama.cpp, opt-in download) | keep it separate; MicroBrain never needs it and never loads it |
| one "Intelligence" screen per feature in Settings | Settings already has Privacy > Patterns, Power, Assistant | a new Settings > Intelligence page with sub-sections; reuse existing pages where the feature already lives |

Not doing: anything that sends data off the machine, an "AI that runs
commands as root", unrestricted macro recording of the screen, killing
user processes to "free RAM", claims we can't back with a measurement.

## 2. Ground rules (from the brief; all still apply)

- [ ] Opt-in learning; pause, inspect, delete everything (Settings).
- [ ] No passwords, keystrokes, clipboard, document contents or browser history collected.
- [ ] Every root change goes through `nexa-brokerd` (its allowlist grows only where needed). No new root daemon.
- [ ] Event-driven: PSI triggers, netlink, inotify/fanotify, udev, D-Bus signals. No busy loops; any polling is slow and stops when idle.
- [ ] Bounded everything: queues, caches, logs, database size, retries, timeouts.
- [ ] Each feature can be switched off on its own; MicroBrain off = NexaOS behaves as it does today.
- [ ] Under memory pressure MicroBrain does *less*, first.
- [ ] Never report a number we didn't measure. Unverified = labelled unverified.
- [ ] Code: Rust for the service, Python only as thin UI (AGENTS.md); no new UI framework.

## 3. Phase 0: baseline before any change

- [ ] Run and record: `make test`, `sh scripts/build-native.sh test`, `make test-files-app test-titlebar test-locker test-nexa-link test-nexa-link-ui test-topbar test-files-drag`, the app sweep, and `scripts/test-live-boot.py` (health report).
- [ ] Measure today's idle RSS/PSS of the per-user services (`nexa-patternd`, `nexa-kerneld`, `nexa-linkd`, `nexa-doctor watch`, shell parts) in a 4 GB QEMU VM: the starting line MicroBrain is judged against.
- [ ] Write the integration map: which service owns which data, events and actions (docs/MICROBRAIN.md).

## 4. Core: `tools/nexa-microbrain` (Rust)

- [ ] Event collector: app launch/exit and focus (from `nexa-kerneld focus-watch` and `nexa-patternd`, not a second watcher), PSI (cpu/memory/io), `/proc/meminfo`, disk free, battery, network (netlink), USB (the broker's `/run/nexa/drives.json`), package transactions (broker audit), settings changes.
- [ ] Event router with typed messages and bounded queues; drop-oldest + a counter on overflow.
- [ ] Decision engine: rules, thresholds with hysteresis, confidence, user preferences, policy checks.
- [ ] Memory engine: local store with migrations, retention limits, size cap, corruption handling (rebuild from scratch rather than crash).
- [ ] Action engine: every action declared (id, argument validation, permissions, approval needed, timeout, rollback if any, audit line). No shell strings.
- [ ] Agent coordinator: the five agents (section 5, feature 8) as components in one process.
- [ ] IPC: `$XDG_RUNTIME_DIR/nexa-microbrain.sock`, 0600, peer uid checked (`nexa_sys::peer`), JSON lines like the other services.
- [ ] Landlock-confined like `nexa-patternd`; Nexa Doctor restarts it, with a crash-loop limit.
- [ ] Resource profile chosen at start from RAM: 2 GB / 4 GB / 8 GB+ (section 8), overridable in Settings.

## 5. The 12 features

Each: **exists** = what we build on; **build** = new work; **test** = done when.

### 1. HabitCore (habit learning)
- Exists: `nexa-patternd` journal, decaying time and sequence models, ranker, "For you" card, Settings > Privacy > Patterns (forget, export, pause).
- Build: [ ] co-occurrence of apps opened together in a time window → "Save these as a workspace?" (feeds PocketWorkspace); [ ] remember dismissed suggestions so they don't come back; [ ] contextual settings (brightness/volume per time/place, already planned as Patterns P4).
- Test: [ ] simulated repeated sequences give the suggestion, random or thin data doesn't; [ ] Forget removes the prediction state.

### 2. SmartFreeze (pause background apps)
- Exists: `nexa-kerneld` user cgroups and the app-in-front signal.
- Build: [ ] freeze (cgroup v2 `cgroup.freeze`) a background app's scope under pressure or on request; [ ] never: compositor, session, audio while playing, downloads in progress, remote sessions, anything on the user's "never" list, system services; [ ] thaw on focus, on Dock click, and by a watchdog after N minutes; [ ] tell the truth: freezing saves CPU, not RAM.
- Test: [ ] a test workload stops and resumes; [ ] protected processes refuse to freeze.

### 3. FutureLoad (pressure prediction)
- Exists: PSI triggers and OOM-kill events in `nexa-kerneld`; Pulse shows CPU/memory/disk.
- Build: [ ] EMAs + trend over PSI, available RAM, swap-in rate, disk free, per-app growth; [ ] predictions with confidence; [ ] hysteresis and a quiet period so wobbling numbers never spam; [ ] "about 1.2 GB of caches may be recoverable" (estimate, said as one).
- Test: [ ] simulated rising memory pressure → warning and mitigation; [ ] fluctuating input → no notification storm.

### 4. PocketWorkspace (save and restore a working set)
- Exists: the compositor layer (`nexa_common.compositor`) lists and places windows; app ids match .desktop files.
- Build: [ ] save: apps (by .desktop id), folders, documents the app can reopen, window positions where the compositor allows; [ ] restore with a report of what couldn't come back; [ ] manifests validated on load, never run as commands; [ ] Workspace Manager in Settings and in Nexus ("Activate coding workspace").
- Test: [ ] save Editor + Terminal + Files on a project folder, close all, restore, all back.

### 5. MiniSkill (user automations)
- Exists: broker actions, Files engine (copy with no overwrite, undo), drives events, notifications (mako).
- Build: [ ] triggers: app launched, USB drive connected, network up, workspace activated, time of day, low storage, file appears in a watched folder, login; [ ] conditions; [ ] actions: open app/folder, copy files (Files engine), move after approval, backup, notification, activate workspace, approved setting, a broker action; [ ] Automation Studio (Trigger → Conditions → Actions), dry run, history; [ ] loop protection and bounded retries; nothing destructive without a yes.
- Test: [ ] "When my backup stick is connected, offer to back up Documents": prompt appears, approved files copied.

### 6. StorageDNA (storage analysis and cleanup)
- Exists: `nexa-tools` duplicates (by hash) and tidy plans; Trash with restore.
- Build: [ ] categories: temp, caches (`~/.cache`, apk cache via broker), old downloads, large files, app data; [ ] incremental metadata index, hashing deferred and size-filtered; [ ] hard links counted once, symlinks not followed, one filesystem; [ ] "recoverable" vs apparent size; [ ] every suggestion says why; deletion goes to Trash.
- Test: [ ] a test tree with duplicates, same-name-different-content files, hard links and caches gives exactly the right findings.

### 7. MicroGuard (security monitoring)
- Exists: the broker refuses non-admins and invalid requests and writes an audit log; `nexa-doctor security`; nftables firewall; Landlock everywhere.
- Build: [ ] read broker audit events; [ ] rules: repeated refusals, a workflow asking for something sensitive, unexpected service/package changes; [ ] Security Center page: event, who, what rule fired, allow/deny where we really can; [ ] never say "blocked" for something we only saw.
- Test: [ ] a disallowed broker request is refused, logged and shown; [ ] an unprivileged program can't pose as the broker (socket owner, peer creds) or skip authorization. Folded into Nexa Defender (section 6).

### 8. NanoAgents (five small agents, one process)
- Build: [ ] Performance, Security, File, Maintenance, Learning agents as components with typed messages, bounded queues, timeouts and permission checks; [ ] no reasoning loops, no shell; [ ] Agent Control Center page (what each did and why).
- Test: [ ] synthetic low-memory event → scan deferred, foreground app untouched, the UI explains it.

### 9. OfflineFirst (network-aware transfers)
- Exists: Store downloads (signed, checked), Nexa Link transfers.
- Build: [ ] online / local-only / metered detection (iwd + routes); [ ] a transfer queue with HTTP range + ETag resume where servers allow, restart otherwise; [ ] backoff, bandwidth and storage limits; [ ] never overwrite on conflict.
- Test: [ ] interrupt a test download, restore network, resumed or safely restarted, checksum matches.

### 10. RecoveryMap (change timeline and undo)
- Exists: broker audit log (installs, services, settings it applied), apk world file, Files' undo.
- Build: [ ] timeline from the broker audit + apk transactions; [ ] config backups taken by the broker before it changes a file; [ ] restore only what's really restorable (config backups, apk add/del reversal); [ ] explicit approval; honest "can't be undone" where so.
- Test: [ ] change a test config through the broker, see it on the timeline, restore, file matches the original byte for byte.

### 11. ZeroSearch (plain-language commands)
- Exists: Nexus (apps, settings, sums, "Ask Nexa").
- Build: [ ] a compact command index: intents, synonyms, fuzzy matching, typo tolerance; every result maps to a registered action; ambiguous → choices; unsupported → a clear "I can't do that yet"; [ ] "Why is my computer slow?" answered from FutureLoad + Pulse data; [ ] lives inside Nexus, no new window.
- Test: [ ] a set of commands, typos and ambiguous phrases routes correctly; unsupported ones are refused politely.

### 12. AdaptivePulse (resource modes)
- Exists: `nexa-kerneld` CPU floor for the app in front and the AI cgroup cap; power profiles in Settings > Power.
- Build: [ ] Balanced / Low resource / Battery saver / Performance / Automatic; [ ] applied through cgroup weights, nice/ionice, deferring our own background jobs (indexing, scans, preloading); [ ] never kill a user process; never touch CPU frequency except through existing kernel interfaces.
- Test: [ ] high memory pressure → background work waits; pressure gone → it resumes.

## 6. Nexa Defender (new: protecting the system and the PC)

Not an antivirus; a rule-based guard built on what Linux can really see.
Monitoring and enforcement are kept apart, and the UI says which is which.

- [ ] Integrity watch (root side, in the broker or a small OpenRC helper): hashes of the files that hand out power: `/etc/passwd`, `/etc/shadow` (metadata only), `/etc/group`, `/etc/doas.conf`, `/etc/pam.d`, `/etc/ld.so.preload`, OpenRC runlevels, `/etc/nftables.nft`, kernel cmdline; alert on change not made through the broker.
- [ ] Persistence watch (user side): new autostart entries, `~/.config/{autostart,sway,hypr}`, shell rc files, crontabs, `~/.ssh/authorized_keys`; show what changed and offer to undo.
- [ ] Odd processes: programs running from `/tmp`, `/dev/shm` or Downloads, deleted-on-disk executables, new listening ports (`/proc/net/{tcp,udp}`), outbound connections from apps without the network permission (where `nexa-run` already denies them, count the denials).
- [ ] Login and authorization: bursts of failed unlocks (lock screen PAM), broker refusals, doas failures → a warning and a short lockout where we enforce it.
- [ ] Downloads: executables and `.nexa` packages from unknown sources flagged; unsigned `.nexa` already refused by the Store engine.
- [ ] USB: a new keyboard-class device appearing while locked (BadUSB-style) is ignored until unlock (udev rule + lock state).
- [ ] Quarantine = move to a root-owned folder through the broker, restorable; never silent deletion.
- [ ] Security Center page (shared with MicroGuard): status (firewall, Secure Boot, disk encryption, updates), recent events, what to do.
- [ ] Tests: each rule fires on a planted change and stays quiet on normal use; nothing here can be used to get root.

## 7. Files app: better interface and more features

The Files app already has: grid and list, thumbnails, Ctrl/Shift/box selection,
drag and drop (fixed today for several files), cut/copy/paste with undo,
Trash with restore, properties, Send to a Device, hidden-root folders hidden.

Interface:
- [ ] Sidebar: Home, Desktop, Documents, Downloads, Pictures, Music, Videos, Recent, Starred, Trash, drives (with eject), Nexa Link devices.
- [ ] Clickable path bar (breadcrumbs) with type-to-edit.
- [ ] Tabs, and a split view for moving between two folders.
- [ ] Preview pane (images, PDF first page, text, audio/video info) and Space for a quick look.
- [ ] Group by type/date, sort menu, per-folder view memory.
- [ ] A storage bar for the current drive.
- [ ] Keyboard shortcut sheet (F1 / Ctrl+?).

Features:
- [ ] Search: by name as you type in the folder; deep search by name and (opt-in) contents through StorageDNA's index.
- [ ] Batch rename (pattern, numbering, find/replace) with preview and undo.
- [ ] Compress / extract (Archive app engine) from the menu.
- [ ] Starred and tags (colour labels), stored in an xattr or a small index.
- [ ] Recent files (from Patterns' file history, opt-in).
- [ ] "Open Terminal here" (exists), "Open as administrator" only via the broker for allowed paths.
- [ ] Network locations (SMB/SFTP) later, through gvfs if it fits in RAM.
- [ ] Conflict dialog on paste: Keep both / Replace / Skip, with "apply to all".
- [ ] Bugs to check while there: rename of a file being thumbnailed; very large folders (10 000+ items) stay responsive; drag to the desktop and to the Dock.

## 8. Resource profiles

| | 2 GB | 4 GB (main target) | 8 GB+ |
|---|---|---|---|
| Features | learning, monitoring, workspaces, ZeroSearch, essential security, rule agents | all 12, conservative scheduling | all 12, more history and concurrency |
| Scans | rare, small boundaries | incremental, deferred | incremental + more indexing |
| History kept | short | medium | long |
| Background jobs at once | 1 | 2 | 4 |
| AI model | never | never by default | optional, separate (Nexa Mind) |

- [ ] zram swap sized by RAM (big win on 2-4 GB). The kernel already has `CONFIG_ZRAM=y`, `CONFIG_PSI=y` and the cgroup freezer, so no kernel rebuild is needed for this, FutureLoad or SmartFreeze.

## 9. Our own ideas

- [ ] "Why is my computer slow?" one-page answer: top memory/CPU users, pressure, swap, disk, what MicroBrain already did (Pulse + ZeroSearch).
- [ ] Battery health (cycle count, design vs full capacity) in Settings > Power and ZeroSearch.
- [ ] Focus mode: Do not disturb + freeze distracting apps for a time, from Nexus.
- [ ] Updates scheduled for when the PC is usually idle and on mains (HabitCore + broker `system_update`).
- [ ] Boot report: what made this boot slower than usual (OpenRC timings + preload).
- [ ] A low-RAM guard before the kernel OOM killer: warn and offer to close or freeze, using PSI (`nexa-kerneld`).
- [ ] Nexa Link + MicroBrain: "your phone's battery is low" and workspace hand-off between two NexaOS PCs.

## 10. Settings > Intelligence (and where each screen lives)

- [ ] Overview: on/off, agents, recent suggestions and decisions, MicroBrain's own RAM/CPU.
- [ ] Learning & Privacy (extends Privacy > Patterns): what's learned, retention, delete.
- [ ] Resource Intelligence: CPU, RAM, pressure, disk, trends, mode (AdaptivePulse).
- [ ] Automation Studio.
- [ ] Storage Intelligence.
- [ ] Security Center (MicroGuard + Nexa Defender).
- [ ] Recovery Timeline.
- [ ] Workspace Manager.
- [ ] Agent Control Center.
- [ ] ZeroSearch lives in Nexus.

Every screen: loading, empty, error and "off" states; no busy redraws; works at 1366x768 and smaller; nothing on the UI thread that can block.

## 11. Measuring and testing

- [ ] Unit tests per module (event parsing, validation, learning rules, thresholds, workflow validation, paths, IPC, permissions, migrations).
- [ ] Integration tests as headless desktop tests (the pattern of `make test-nexa-link-ui`).
- [ ] Security tests: malformed IPC, wrong peer uid, path traversal, symlinks, workflow loops, oversized events, broken database rows.
- [ ] Reliability: crash and restart, queue overflow, cancellation, timeouts, missing hardware.
- [ ] A 4 GB and a 2 GB QEMU profile: idle RSS/PSS, idle CPU (normalised to all cores, stated), disk writes per hour idle, event latency, behaviour under `stress-ng` memory/CPU load. Numbers go in docs/MICROBRAIN.md with the machine and the method.
- [ ] Real hardware (HP 840 G6, TM1): marked unverified until run there.

## 12. Order of work

1. [ ] Phase 0 baseline (section 3).
2. [ ] Core service + IPC + store (section 4).
3. [ ] FutureLoad, AdaptivePulse, NanoAgents, HabitCore additions.
4. [ ] PocketWorkspace, MiniSkill, ZeroSearch.
5. [ ] SmartFreeze, StorageDNA, OfflineFirst.
6. [ ] MicroGuard + Nexa Defender, RecoveryMap (broker additions).
7. [ ] Files app work (section 7) alongside, in small commits.
8. [ ] Settings > Intelligence screens.
9. [ ] Measurements, docs (architecture, install, config, features, security, performance, troubleshooting, developer guide), one ISO build, test on the HP.

Each step ends with its tests passing and a commit; nothing is ticked on files alone.
