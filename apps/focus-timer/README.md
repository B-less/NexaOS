# Focus Timer

Work on one thing for a set time, then take a break. Finished sessions are
kept in `~/.local/share/focus-timer/sessions.json` (the last 200).

Written in NexaLang (`nexalang/NEXALANG.md`), the reference for NexaOS apps
in that language. No permissions: it only uses its own data folder.

- Build: `nexa-pack build` (needs `nexac` on the PATH)
- Keys: Enter or Ctrl+Return start and pause, Ctrl+R resets,
  Ctrl+Shift+Delete clears the history, Esc or Ctrl+Q closes.
  Enter (or a double-click) on a finished session sets it up again.
- Tests: `tests/*.steps` are driven with real key presses by
  `nexalang/tests/ui/drive.py`, and run by `cd nexalang && cargo test`
  three ways (leak counter, AddressSanitizer + UBSan, Valgrind); `*.out` is
  the exact output expected (the saved history file).
