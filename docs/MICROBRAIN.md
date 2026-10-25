# Nexa MicroBrain

NexaOS's small intelligence service: it learns, predicts, automates and
protects with rules, moving averages, trends and counts, not with an AI
model. One Rust process per user (`nexa-microbrain`), started with the
desktop session. The plan and the feature list: [TODOnexa_inter.md](../TODOnexa_inter.md).

## How it fits together

```
 Settings > Intelligence, Nexus, notifications
            |  $XDG_RUNTIME_DIR/nexa-microbrain.sock (0600, peer uid checked)
      nexa-microbrain (Rust, as you, Landlock-confined)
        sampler ---- PSI trigger on /proc/pressure/memory wakes it;
                     otherwise one sample every 10-30 s (by profile and mode)
        coordinator - FutureLoad (resources.rs) -> AdaptivePulse (pulse.rs)
                      -> agents (agents.rs) -> scheduler (scheduler.rs)
        store ------- ~/.local/share/nexa-microbrain, versioned JSON (store.rs)
            |  anything that needs root
      nexa-brokerd (allowlisted actions, audit log)
```

Working today (2026-10-25): FutureLoad (sampling, levels with hysteresis,
forecasts with confidence), AdaptivePulse (five modes and the budget),
the scheduler, the Performance and Maintenance agents, the store with
migrations, the socket and the command line. The other features follow
the order in TODOnexa_inter.md section 12.

## Commands

```
nexa-microbrain serve                 # the service
nexa-microbrain status                # levels, budget, forecasts, jobs, its own RSS/PSS/CPU
nexa-microbrain mode low-resource     # auto | balanced | low-resource | battery-saver | performance
nexa-microbrain agents | decisions | jobs
nexa-microbrain config freeze on      # a feature on or off
```

## Resource profiles

Picked from MemTotal unless chosen in Settings: under 3 GB is the 2 GB
profile (1 background job, 30 samples of history, a sample every 30 s),
under 7 GB the 4 GB profile (2 jobs, 60 samples, 20 s), above that 8 GB+
(4 jobs, 120 samples, 10 s).

## Measurements

### Baseline before MicroBrain (measured 2026-10-25)

QEMU/KVM, 4 GB RAM, 2 vCPUs, the live ISO built 2026-10-25 14:46, booted
as a USB stick, desktop session (Sway, no GPU), measured at 180 s of
uptime with `/proc/<pid>/smaps_rollup`. PSS splits shared pages between
the processes that use them; that's the fairest per-process number.

| Process | RSS MB | PSS MB |
|---|---|---|
| sway | 63.5 | 36.7 |
| wireplumber | 57.6 | 36.1 |
| Dock (python3 nexa_shell dock) | 49.4 | 28.1 |
| waybar | 53.3 | 26.5 |
| For you card (nexa_shell foryou) | 44.8 | 25.0 |
| Drives on the desktop (nexa_shell drives) | 44.7 | 24.8 |
| Nexa Link pop-ups (nexa_shell linknotify) | 44.3 | 24.6 |
| nexa-locker (lock screen, boot lock still up) | 38.1 | 16.0 |
| nexa-linkd | 1.0 | 1.0 |
| nexa-brokerd (root) | 0.9 | 0.9 |
| nexa-doctor watch | 0.8 | 0.8 |
| nexa-patternd serve | 0.9 | 0.5 |
| nexa-kerneld serve (root) / focus-watch | 0.7 / 0.6 | 0.4 / 0.3 |

MemAvailable at that moment: 3476 MB of 3901 MB.

What it says: NexaOS's Rust services cost about 1 MB each; the four
Python desktop parts cost about 100 MB together, mostly because each is
its own Python + GTK process.

**Done (2026-10-25):** they now run as one process, `nexa-parts`.
Measured in a headless Sway (nexa-apptest container), same parts, same
method: 104 MB PSS as four processes, **39 MB as one: 65 MB saved** on
every NexaOS computer.

### MicroBrain itself

In a clean Alpine container (`make test-microbrain`), sampling every
second (its busiest setting; normally every 10-30 s), idle for 30 s,
measured by the kernel (`/proc/self/smaps_rollup`, `/proc/self/stat`),
2026-10-25: **RSS 0.9 MB, PSS 0.5 MB, CPU 0.1 % of one core.**
Host: 8 cores; the CPU figure is of one core, not normalised.

Not measured yet: inside the 4 GB VM with the whole desktop (needs an ISO
with it), and on real hardware (HP 840 G6, TM1).
