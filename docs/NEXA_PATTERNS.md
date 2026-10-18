# Nexa Patterns — learning how you use the computer, and getting ahead of you

Written 2026-10-18. A plan for NexaOS to learn each person's habits on the
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
| Next app (top 3) | a "Suggested" row in Launchpad, the Dock's first slot hint | yes (only arranges) |
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
           Launchpad / Dock · bar battery menu · Files · assistant tool `my_patterns` · Settings > Privacy > Patterns
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
- Switches: Suggestions · Faster app starts (preloading) · Routines ·
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
| **P2 — Next app** | time habits, Markov chains, the ranker; Launchpad "Suggested" row; Why? / Don't suggest | replaying a synthetic month of usage gives top-3 accuracy ≥ 60% on its last week; explanations match the counts |
| **P3 — Faster starts** | preloader (readahead of predicted apps' executables and libraries, memory-pressure aware); AI model warm-up at usual assistant times | measured cold-start time of a predicted app drops (target −30% on the HP); preloading stops when memory PSI rises |
| **P4 — Battery and brightness** | battery profile and forecast in the bar; learned brightness/volume curves, offered first | forecast error under 20 min on replayed real logs; brightness offer appears only after 10 consistent changes and turns off at the next manual change |
| **P5 — Routines and the assistant** | routine mining when idle; routine offers; `my_patterns` assistant tool and the opt-in context summary | a planted daily routine in synthetic data is found within 5 days; three dismissals retire it; the summary contains no file names or window titles |
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
