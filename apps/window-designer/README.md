# Window Designer

Build a NexaLang window by adding parts to it (headings, text, notes,
buttons, one main button, text fields, check boxes, lists, rows and
columns), see how it will look, and get the NexaLang program for it: a
complete program with the `match ui.wait()` loop and a place for what each
button, text field, check box and list does.

Written in NexaLang (`nexalang/NEXALANG.md`). It uses `ui.clear` to build
the preview, `ui.code` to show the program, `ui.save_file` to export it and
`ui.to_clipboard` to copy it (all new in NexaLang 0.7).

- Build: `nexa-pack build` (needs `nexac` on the PATH)
- Keys: Enter on a part (or Ctrl+Return) adds it, then type its words and
  press Enter to go back to the parts; Alt+Up and Alt+Down move the
  selected part, Alt+Right puts it into the row or column above it,
  Alt+Left takes it out, Alt+Delete removes it; F5 preview, F6 code,
  Ctrl+E export, Ctrl+Shift+C copy, Ctrl+N new design, Esc or Ctrl+Q close.
- The design is kept in `~/.local/share/window-designer/design.json`
  (saved after every change). A file it can't read is never written over:
  the window says why, and New design offers to replace it.
- Permissions: `files.documents`, so Export can save the program in your
  Documents folder (the file chooser runs inside the app, so an installed
  app can only save where its permissions let it). Copy needs nothing.
- Tests: `tests/*.steps` are driven with real key presses by
  `nexalang/tests/ui/drive.py`, and run by `cd nexalang && cargo test`
  three ways (leak counter, AddressSanitizer + UBSan, Valgrind); `*.out` is
  the exact output expected (the window's widgets, printed with
  Ctrl+Shift+F12 in tests, and the files it saved). Every program exported
  in those tests is then checked, formatted-checked, built with the
  hardened flags and run, and must show exactly the widgets the preview
  showed.
