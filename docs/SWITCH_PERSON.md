# Switch person: several people, one computer

Each person who may use the desktop (an account in the `seat` group with a
password) gets a desktop of their own. Switching doesn't sign anyone out:
the first person's windows stay exactly as they were, paused, while the
other person uses the screen.

## Where to switch

- **The lock screen**: everyone else is a hexagon under the password field
  ("SWITCH PERSON"); a green dot means they're signed in. Click one, or press
  Tab (then Tab/arrows to choose) and Enter.
- **Power** (the power menu): "Switch to …" for up to three people.
- **Settings > Users**: "Switch to …" on each person's card, and a
  "Signed in" tag. The Living Field's Accounts object shows who else is
  signed in and has the same buttons.

The desktop you leave is always locked first. The other person's desktop
opens on its own lock screen and asks for their password, even if they
chose not to be asked at startup.

## What happens underneath

```
  lock screen / Power / Settings
        │  nexa-lock --switch-to ama
        ▼
  nexa-locker (C)  locks, snapshots the desktop (wlr-screencopy),
        │          plays the hand-over, then asks:
        │  "switch ama"  on /run/nexa-session.sock
        ▼
  nexa-sessiond (Rust, root, the OpenRC "sway" service)
        │  starts Ama's desktop on a free VT (7-12), or brings hers back
        ▼
  seatd pauses the old compositor, resumes the new one
```

- `nexa-sessiond` checks with the kernel who's asking: only the person whose
  desktop is on screen (or root) can switch. Only people in the `seat` group
  with a usable password get a desktop.
- Any VT change (Switch person, or Ctrl+Alt+F-keys) locks the desktop that
  left the screen, and tells the lock screen of the one that arrived to
  play its arrival (SIGUSR2).
- Signing out of a second person's desktop goes back to the one that was on
  screen before. "Log out" of the first person's starts it again, locked,
  as before. Whatever someone left running is ended when their desktop ends.
- `nexa-sessiond people` lists everyone; `nexa-sessiond switch NAME` switches
  (as root, or as the person on screen).

## The animations

- **Leaving** (1.3 s): the desktop (or the lock screen) breaks into the
  honeycomb from the clock's centre outwards, each cell shrinking away with
  its edge lit; the other person's hexagon warps in inside turning rings
  ("Switching to Ama", and a terminal line typing the hand-over); a hexagon
  iris opens to the edges and the screen goes dark.
- **Arriving** (1.2 s): from dark, the person's hexagon appears ("Welcome
  back"), its outline grows into the clock's hexagon while the honeycomb
  reassembles the lock screen from the middle out.
- `"motion": "none"` in Interface DNA (`~/.config/nexa/interface/global.json`)
  turns both off; the lock screen then just appears.

## Known limits

- Sound: each person's PipeWire runs on its own; the one on screen may not
  get the sound card while the other's holds it (no logind here to hand it
  over). Not tested on hardware yet.
- Nexa Link listens on fixed ports, so only the first desktop to start it
  has Nexa Link.
- The real VT switch is only tested in the VM (`scripts/test-live-boot.py`,
  the USERSWITCH line); containers have no VTs, so `make test-sessions` uses a
  stand-in VT and stand-in desktops.
