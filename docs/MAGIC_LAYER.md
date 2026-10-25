# Magic Layer and the Living Visual Engine

The plan, with what's done and what isn't: [TODOnexa_magic.md](../TODOnexa_magic.md).

## Living backgrounds

```
 Settings > Magic Layer ──► ~/.config/nexa/magic.json ◄── nexa_common.magic (apps: glass, clicks)
                                      │ (watched)
 theme change (lock.conf) ──────────► nexa-living (C, one process per desktop)
 Sway IPC / Hyprland sockets ───────►   ├ background layer: wallpaper + desktop style around windows
 MicroBrain mode, battery, /proc ───►   ├ each Nexa app's style inside its window area
                                        └ $XDG_RUNTIME_DIR/nexa-living.stats (measured)
```

- **One renderer, behind everything.** `nexa-living` draws on the
  compositor's background layer, so it can never cover text or take a click
  from an app. A Nexa app with a living background gets glass (see-through,
  `window_alpha`), so its style shows behind its own content; the renderer
  dims the area first (`veil`) to keep text readable.
- **Ten styles**, in `tools-c/nexa-living/styles.c`: Cyber Circuit, Quantum
  Flow, Neural Network, Holographic Grid, Digital Rain, Cosmic Nebula, Energy
  Pulse, Orbital Field, Living Geometry, Nexa Signature. Each makes its
  geometry once per window size; the parts that never move are drawn once
  into a cache and copied each frame.
- **Only what moves is redrawn**: the focused styled window, and the desktop
  around the windows (at most 15 frames a second). Unfocused windows keep
  their last frame (High: they move slowly). Hidden windows cost nothing.
- **Quality**: High 60 fps, Balanced 30, Low 12, Static (drawn when something
  changes), Off. Automatic is Balanced, or Low on a 2 GB computer. On battery,
  with MicroBrain's Battery saver or Low resource, it drops to Low; with
  reduced motion (or Interface DNA `"motion": "none"`) to Static. Frames that
  take more than half their time for 2 s step it down; 10 s comfortably fast
  steps it back up.
- **Theme**: the colours are the theme's (accent, second accent, text,
  background); changing the theme recolours the backgrounds at once.
- **Apps can add to it** (`nexa_common.magic`): `click(app, x, y)` (a
  ripple), `level(app, 0..1)` (real activity, e.g. load or playback).
- **Other apps** keep the theme's look: their insides can't be changed from
  outside.

### Measured

Headless Sway, pixman (the CPU renderer VMs use), 1366x768, `make
test-living` on the build machine (i5-8365U), 2026-10-25:

| What | Result |
|---|---|
| Balanced, Settings focused + desktop moving | 24 fps, 4.0 ms a frame (copy 1.4, desktop 0.7, window 2.0), 10% of one core |
| Low | 11 fps, 3.2 ms a frame |
| Static | 0% CPU between changes |
| Memory (RSS) | about 21 MB |

One frame of each style at 1000x700 (`nexa-living-preview --bench`, before
the static-part caches: the real renderer is cheaper): see the table in the
commit "Living Visual Engine". Not yet measured: real hardware (Intel UHD 620
with Hyprland), the 2 GB VM, battery drain.

### Files

- `tools-c/nexa-living/`: the renderer (`nexa-living.c`), the styles
  (`styles.c`), a small JSON reader, and `nexa-living-preview` (one frame to
  PNG, or a timing run; Settings uses it for the style preview).
- `common/nexa_common/magic.py`: the settings, defaults, talking to the
  renderer. `settings-app/nexa_settings/magic_page.py`: the page.
- Tests: `make test-living`, `make test-magic-page`, `common/tests/test_magic.py`.
