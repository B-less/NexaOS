# Nexa Patterns — learning how you use the computer, and getting ahead of you

Written 2026-10-18; **phases 1 and 2 built the same day** (see 9). A plan for NexaOS to learn each person's habits on the
laptop itself, adapt to them, and make useful predictions: the app you're
about to open, when the battery will run out, the brightness you'd pick,
the routine you do every morning. Nothing leaves the machine, you can see
and erase everything it has learned, and it never acts on a guess without
showing why.

Builds on what exists: `nexa-kerneld` (process and focus events, power,
pressure), Nexa Mind (the on-device model), Settings > Privacy, the Dock
and Launchpad, and the broker.

**Reference machine:** HP EliteBook 840 G6 (4 cores, 8 GB). Budget for the
whole feature: **under 10 MB of memory, under 1% of one core on average**,
learning work done only when the laptop is idle and calm (PSI low).

---

## 1. Principles

1. **Opt-in, per person.** Off until you switch it on (first-run screen,
   then Settings > Privacy > Patterns). Each account learns only its own
   habits; nothing is shared between accounts.
2. **Small, explainable models, not the language model.** Habits are
   learned with counting and simple statistics that run in microseconds
   and can always say *why* ("you opened Music around 8:30 on 9 of the last
   10 weekdays"). The language model only *reads* a short summary, when you
   allow it.
3. **Predict freely, act carefully.** Predictions that only arrange things
   (a "Suggested" row, preloading an app into memory) happen silently.
   Anything that changes something you'd notice (brightness, opening apps,
   power mode) is offered first, and becomes automatic only after you say
   yes to it.
4. **Forgetting is built in.** Old habits fade (exponential decay, 14-day
   half-life by default); raw events are kept 14 days, then only counts
   remain. "Forget this app", "Forget the last hour" and "Forget everything"
   are one click each.
5. **Never learned:** private browser windows, the lock screen, password
   prompts, anything in an app you mark "don't learn from this", file
   *contents*, what you type.

---

## 2. What it observes

All of it already exists on the system; nothing new is collected from the
network or from inside apps.

| Signal | Source | Stored as |
|---|---|---|
| App launched / closed | `nexa-kerneld` process connector, filtered to the person's uid | app id, time |
| App in front, and for how long | the session focus watcher (`nexa-kerneld focus-watch`) | app id, start, duration |
| Time context | clock | hour (in 30-min slots), weekday / weekend |
| Place context | the Wi-Fi network you're on (iwd), **hashed**, never the name | an opaque place id ("place 2") |
| Power context | battery %, charging, drain in watts (`nexa-kerneld`) | per-minute summary |
| Your manual settings changes | brightness / volume / power mode / Do Not Disturb, as they go through the broker or the volume service | old → new value + context |
| Files you open (name only, never contents) | the app that opened it reports it (GTK recent-files list) | path hash + folder + app |
| Assistant use | the assistant | time, topic group (not the text) |
| Suggestion feedback | Dock / Launchpad / notifications | shown, used, dismissed |

---

## 3. How it learns (the models)

All in Rust, online (each event updates the model at once), with
exponential decay so recent weeks count more than old ones.

| Model | Learns | Method | Used for |
|---|---|---|---|
| **Time habits** | which apps you use at which time, on which kind of day, in which place | decayed counts per (app, 30-min slot, weekday/weekend, place) | "Suggested" apps; preloading |
| **Sequences** | what you open after what ("Terminal, then Files, then Editor") | 1st- and 2nd-order Markov chains over app switches, decayed | next-app prediction |
| **Next-app ranker** | combines the two above with recency and how long ago you last used each app | online logistic regression (a few dozen weights per app, trained by SGD on "what did they actually open next") | top-3 suggestions |
| **Routines** | things you do at about the same time most days | circular statistics on time-of-day (mean direction and resultant length of an app's start times) plus frequent-sequence mining over morning/evening sessions (a small PrefixSpan), when idle | "Every weekday at 9 you open Mail, Browser and Notes: open them together?" |
| **Battery profile** | how fast *you* drain the battery at each hour of the day, per app mix | decayed average watts per hour slot and per foreground app | "Battery lasts until about 15:40 at your usual pace"; charge reminders |
| **Preferences** | the brightness / volume you choose in each context | per-context piecewise-linear curves fitted to your manual changes (like phones' adaptive brightness) | learned brightness and volume, offered first |
| **Focus time** | when you work long uninterrupted stretches | session lengths by hour | quiet hours for notifications, background work scheduled outside them |

**Cold start:** no predictions for the first 3 days, or until an app has
been seen 5 times. **Confidence:** every prediction carries a probability.
Below 0.35 it isn't shown; below 0.6 it only arranges things (no offers).
**Feedback:** a dismissed suggestion counts as a negative example; three
dismissals of the same routine retire it.

---

## 4. What it does with the predictions

| Prediction | What you notice | Acts silently? |
|---|---|---|
| Next app (top 3) | **on the desktop** (see 4a), a "Suggested" row in Launchpad, the Dock's first slot hint | yes (only arranges) |
| Next app, high confidence | **the app opens faster**: its program and libraries are read into memory ahead of time (`readahead` / `posix_fadvise(WILLNEED)`, done as you, no root), and `nexa-kerneld` gives it the CPU floor the moment it starts | yes (only uses idle time and free memory; dropped under memory pressure) |
| You usually ask the assistant now | the AI model is loaded into memory a minute before, unloaded when you usually stop (saves the 5–10 s load) | yes, only on mains power or above 50% battery |
| Routine found | an offer: "Open Mail, Browser and Notes together at 9:00?" | no: becomes automatic only after "Yes" |
| Battery forecast | "At your usual pace the battery lasts until 15:40" in the bar's battery menu; a reminder to charge before a long unplugged stretch you usually have | shows only |
| Learned brightness / volume | offered after 10 consistent manual changes: "Adjust brightness like this automatically?" | no: automatic only after "Yes", undone by any manual change |
| Focus time | notifications held during your usual focus stretches (if you turned on "Learn my quiet hours") | no: opt-in |
| Files | the Files app's "Likely next" folders; the assistant's "the report you usually open on Mondays" | yes (only arranges) |
| The assistant knows you | a 10-line summary of your habits added to its context ("usually works 9–17, Terminal and Editor; asks about the weather in the morning") | only if "Let the assistant use my patterns" is on |

Every suggestion has a **Why?** that shows the evidence, and a
**Don't suggest this** that removes it.

### 4a. Suggestions on the desktop

A "For you" card on the desktop itself, drawn by the shell
(`nexa_shell foryou`, launcher `nexa-foryou`; GTK + gtk-layer-shell on the **bottom layer**: above
the wallpaper, below every window, so it's there when the desktop is
visible and never covers your work). Works the same in Sway and Hyprland.

```
 ┌ For you ───────────────────────── 08:42 ┐
 │  ⬡ Mail     ⬡ Browser    ⬡ Notes         │   apps you'll likely open next
 │  Weekday morning: open all three?  [Open]│   a routine (only once it's found)
 │  📄 report-q3.odt (Documents)            │   the file you usually pick up now
 │  🔋 Lasts until about 15:40               │   battery forecast, when unplugged
 └──────────────────────────────────────────┘
```

- **Changes with the moment:** recomputed when you come back to the desktop,
  every 15 minutes, and when the context changes (plugged in, another
  network, morning → afternoon). It shows at most 3 apps, 1 routine, 1 file
  and 1 forecast, only those above the confidence threshold. Nothing
  confident to say: it shows nothing (no filler).
- **Mouse:** click an app or file to open it; [Open] runs the routine;
  hover for **Why?** ("opened at about this time on 9 of the last 10
  weekdays"); right-click: Don't suggest this · Why? · Hide for today ·
  Move the card (top-left, top-right, bottom-right, centre).
- **Hexa design:** a translucent glass card in the theme's `@nexa_*`
  colours (blurred on Hyprland), hexagon app badges, Inter, a soft fade
  when it updates; follows light/dark and the accent colour.
- **Costs next to nothing:** one sub-millisecond question to `nexa-patternd`
  every 5 minutes (and when Patterns settings change); between them it
  sleeps.
- **Feedback:** clicks count as "used", right-click "Don't suggest" counts
  as a negative example (section 3), so the card learns what's useful.
- **Off switch:** Settings > Privacy > Patterns > "Show suggestions on the
  desktop" (on by default once Patterns is on), plus the position.

---

## 5. Architecture

```
 kernel ── nexa-kerneld (root) ── per-user event stream ──┐
 session ─ focus-watch, Dock, Files, assistant, Settings ─┤
                                                          ▼
                     nexa-patternd  (Rust, runs AS the user, in the session)
                     ├─ journal: append-only binary log, 14 days   (~/.local/share/nexa-patterns/)
                     ├─ models: counts, chains, weights (serde, versioned)
                     ├─ learner: online updates; nightly mining when idle (PSI-aware)
                     ├─ preloader: readahead of predicted apps' files (as the user)
                     └─ socket $XDG_RUNTIME_DIR/nexa-patterns.sock
                          predict(next_app | battery | routine | brightness), explain, feedback,
                          forget(app | since | all), export, pause
                                                          ▼
  desktop "For you" card · Launchpad / Dock · bar battery menu · Files · assistant tool `my_patterns` · Settings > Privacy > Patterns
```

- **Per user, not root.** `nexa-patternd` runs as the person, Landlock-
  confined to its own data folder (read-only to app files for preloading).
  The root daemon only forwards that uid's own process events (a new
  `subscribe` op in `nexa-kerneld`, checked with SO_PEERCRED).
- **Storage.** Files 0600 in the home folder; the journal is a compact
  binary log (about 40 bytes per event, around 2 MB for 14 days); models are
  a few hundred kB. Encryption at rest comes with disk encryption
  (docs/HARDENING.md) rather than a separate key.
- **No new kernel options** for phases 1–5. Phase 6 (file-open patterns for
  every app, not just GTK's recent list) would need `FANOTIFY`.

---

## 6. Settings > Privacy > Patterns

- On/off, and Pause for an hour or a day.
- "What NexaOS has learned": your top routines, time habits per app,
  battery profile, learned brightness, each with its evidence.
- Per app: "Don't learn from this app", "Forget this app".
- Forget the last hour / day / everything; Export (JSON) to see the raw data.
- Switches: Suggestions · Show suggestions on the desktop (and where) ·
  Faster app starts (preloading) · Routines ·
  Learned brightness and volume · Quiet hours · Let the assistant use my
  patterns.
- How well it's doing: "Suggestions were right 7 times in 10 this week".

---

## 7. Phases

Each phase ships with tests and doesn't need the next one. Per the build
rule, the ISO is built once for the whole batch.

| Phase | Builds | Done when (tests) |
|---|---|---|
| **P1 — Journal and controls** | `nexa-patternd` (journal, decay, forget, export, pause); `nexa-kerneld` `subscribe` (own-uid events only); Settings > Privacy > Patterns (on/off, forget, export) | events recorded only when on; forget really deletes (journal and counts); private windows and excluded apps never recorded; another account's events never arrive |
| **P2 — Next app, on the desktop** | time habits, Markov chains, the ranker; the desktop "For you" card (apps, then files) and Launchpad's "Suggested" row; Why? / Don't suggest / Hide for today | replaying a synthetic month of usage gives top-3 accuracy ≥ 60% on its last week; explanations match the counts; the card shows nothing below the threshold, sits under windows (headless Sway screenshot test), and a dismissed app doesn't come back |
| **P3 — Faster starts** | preloader (readahead of predicted apps' executables and libraries, memory-pressure aware); AI model warm-up at usual assistant times | measured cold-start time of a predicted app drops (target −30% on the HP); preloading stops when memory PSI rises |
| **P4 — Battery and brightness** | battery profile and forecast in the bar and on the desktop card; learned brightness/volume curves, offered first | forecast error under 20 min on replayed real logs; brightness offer appears only after 10 consistent changes and turns off at the next manual change |
| **P5 — Routines and the assistant** | routine mining when idle; routine offers on the desktop card; `my_patterns` assistant tool and the opt-in context summary | a planted daily routine in synthetic data is found within 5 days; three dismissals retire it; the summary contains no file names or window titles |
| **P6 — Wider view (optional)** | file-open patterns for every app (`FANOTIFY`), Files "Likely next" folders | needs a kernel option; done only if P1–P5 prove useful |

**Evaluation tool:** `nexa-patternd replay <journal>` runs the models over
a recorded or synthetic journal and prints accuracy, calibration
(predicted vs actual), and forecast error, so every change to the models
is measured before it ships.

---

## 8. Risks and how they're handled

| Risk | Handling |
|---|---|
| Creepy or wrong suggestions | confidence thresholds, cold start, Why?, one-click "Don't suggest", decay so old habits fade |
| A habit log is sensitive if someone gets the laptop | opt-in; minimal data (app ids, hashed places, no contents or titles); 14-day raw retention; forget buttons; disk encryption (HARDENING.md) |
| Preloading wastes memory or battery | only free memory, dropped under memory pressure, off on low battery, never more than 3 apps |
| Feedback loops (suggesting X makes you open X, which makes it more likely) | launches that came from a suggestion count half; accuracy measured against launches *not* prompted by it |
| Shared accounts | per-account models; Pause; "Guest" account never learns |

---

## 9. What's built (phases 1 and 2)

| Piece | Where |
|---|---|
| `nexa-patternd` (Rust): the session service, journal, models, explanations, forget/export, replay | `tools/nexa-patternd` |
| Desktop "For you" card | `shell/nexa_shell/desktop.py` (`nexa-foryou`) |
| Nexus "Suggested for you" row | `shell/nexa_shell/nexus.py` |
| Settings > Privacy > Patterns | `settings-app/nexa_settings/patterns_section.py` |
| Client and settings file | `common/nexa_common/patterns.py`, `~/.config/nexa/patterns.json` |

Differences from the plan above, found while building it:

- **Apps come from the compositor, not the kernel link.** Every NexaOS app
  runs as `python3`, so process names can't tell them apart; Sway's and
  Hyprland's focus events carry the Wayland app id (`nexa-files`, `foot`),
  which also matches the app's `.desktop` file. So `nexa-kerneld` needed no
  `subscribe` op.
- **An app counts after 5 seconds in front**, so flicking past windows isn't
  learned.
- **Confidence threshold:** suggestions show at 12% or more (the ranker's
  probability that this app is *the* next one, out of all apps). On the
  synthetic month the true next app is the top guess 70% of the time and in
  the top 3 80% of the time (`nexa-patternd replay --synthetic 35`).
- **Places** are the default router's hardware address, hashed (from
  /proc/net/route and /proc/net/arp), so no Wi-Fi name is ever read.
- **Files on the card** come straight from GTK's recent-files list
  (`recently-used.xbel`), read when the card asks; file names are never
  copied into the journal.
- Private and Tor windows of Nexa Browser now keep "— Private" / "— Tor"
  at the end of their title (it used to be replaced by the page's title), so
  they're recognised; Firefox and Chromium private windows are too.

Tests: `cargo test -p nexa-patternd` (journal, decay, models, explanations,
time zones, the replay accuracy bar, a fake Sway session: off until on,
private/excluded/flicked windows never recorded, titles never kept,
forgetting deletes), `common/tests/test_patterns.py` (the client against the
real daemon) and `make test-desktop-card` (the card and Nexus in a headless
Sway, with screenshots).

## 10. Phase 3: faster starts (built)

- **What an app needs:** when an app has been in front for 5 s, the
  daemon notes the system files it has loaded - its program and libraries
  from `/proc/<pid>/maps`, and for NexaOS's Python apps their package plus
  `nexa_common` and `gi` - in `~/.local/share/nexa-patterns/apps/<app>.files`
  (system folders only, refreshed weekly). Landlock forbids a confined
  process from reading other processes' maps, so a small helper started
  before confinement (`nexa-patternd maps-helper`) answers only "pid ->
  loaded system files".
- **Getting ahead:** every minute, apps at least 30% likely to be next and
  not already running are read into the page cache (readahead(2)), at most
  once per half hour each, up to 300 MB per app - only with a quarter of RAM
  left free, no memory pressure, and on mains or 30%+ battery. Settings >
  Privacy > Patterns > "Faster app starts" switches it off.
- **The assistant:** when it's likely next (and on mains or 50%+ battery),
  `nexa-mindd` is asked to load the model (`POST /v1/warm`), saving the
  5-10 s load; it unloads after the usual idle time if not used.
- `nexa-patternd` `{"op": "warm", "app": ...}` does the same on demand;
  `status` lists what was warmed and how much.

Tests: `cargo test -p nexa-patternd` (a real process's files captured
through the helper, system files only; warming reads them into memory; the
assistant's warm-up reaches a fake Nexa Mind), and the unit tests of
`nexa_sys::preload` (mincore/readahead, the memory guard).
