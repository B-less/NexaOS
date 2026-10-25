# The Living Field: interaction grammar

The plan and phases: [TODOnexa_living.md](../TODOnexa_living.md).

## The idea

Settings usually asks you to know where something lives ("is battery
under Power or System?"). The Living Field shows the computer as the
things it's made of, each one a hexagon cell that tells you its state at
a glance, grouped by what they're about, and joined to what they affect.
You go to the thing, not to the menu.

```
   YOUR SYSTEM          CONNECTIONS           YOU                 WORK              SAFETY
   ⬡ Performance ●──┐   ⬡ Network ●───┐      ⬡ Appearance ●      ⬡ Workspaces ●    ⬡ Security ●
     ⬡ Storage ▲ ───┤     ⬡ Bluetooth ■│        ⬡ Privacy ●──────┼─⬡ Intelligence ●
   ⬡ Power ●     ───┘   ⬡ Nexa Link ●─┘      ⬡ Accounts ●        ⬡ Apps ●
     ⬡ Updates ●                               ⬡ Time ●
```

## Grammar

- **Object** (NexaObject): one capability of the computer. A hexagon with
  its name, a one-line live summary ("3.1 of 3.9 GB used"), and a state
  shape: ● fine, ▲ needs attention, ■ off. Shape and colour together,
  never colour alone.
- **Cluster** (NexaCluster): objects about the same part of life, always
  in the same place. Pinned objects get their own cluster at the start;
  nothing else moves.
- **Thread** (NexaThread): a thin line from an object to the ones it
  affects (Performance ↔ Storage, Power, Intelligence; Network ↔ Nexa
  Link, Security; Privacy ↔ Intelligence). Drawn faintly; select an
  object and its threads and partners light up.
- **Three depths**:
  1. *Compact*: the cell itself (name, summary, state).
  2. *Open* (NexaMorph): the cell opens a panel beside the field with
     the object's real controls (Performance: the mode and why; Appearance:
     theme swatches; Bluetooth: on/off...). The field stays visible: you
     don't lose your place.
  3. *Full* (NexaPortal): "All settings" goes to the full page or app.
     Back returns to the field with the same object selected.
- **Command** (NexaCommand): type anywhere; objects that match stay lit,
  the rest dim; Enter opens the best one. ZeroSearch commands work here
  too.

## Keyboard and access

Arrow keys move between objects by where they are on screen; Enter or
Space opens; Escape closes the panel, then clears the search; Ctrl+L
switches to the list (the sidebar), which reaches every function. Every
object is a real button with an accessible name ("Performance: no memory
pressure, Balanced mode") and a visible focus ring.

## Motion and cost

No motion in reduced-motion mode or the 2 GB profile; otherwise a single
120 ms fade when the panel opens. Threads are drawn once per layout,
redrawn only when the selection changes. Live summaries refresh every 5 s,
only while the field is on screen; each refresh runs off the UI thread
with short timeouts.
