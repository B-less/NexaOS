# Code Studio

Write NexaLang programs. Mistakes show up as you type (underlined, and
listed under the program with what to do about them), Lay out puts the
program in NexaLang's standard layout, Explain says what a mistake means
with a refused and a fixed example, and Run builds and runs the program
and shows what it prints. A program that never stops can be stopped.

Written in NexaLang (`nexalang/NEXALANG.md`), with the `editor` module
(GtkSourceView 4 with NexaLang's colours, new in NexaLang 0.8) and
`ui.start_program`. Everything about the language comes from `nexac`
itself, started with an argument list, never a shell:

| What | How |
|---|---|
| Mistakes as you type | `nexac check --json --stdin FILE.nx`, 0.4 s after the last key press |
| Lay out | `nexac fmt --stdin` (one step: Ctrl+Z puts it back) |
| Explain | `nexac explain --json TOPIC` (the topic `check --json` gave) |
| Run | `nexac run FILE.nx`, in the file's folder (saved first) |

- Build: `nexa-pack build` (needs `nexac` on the PATH).
- Keys: Ctrl+O open, Ctrl+S save, Ctrl+N new, Ctrl+Return or F5 run,
  Shift+F5 stop, Ctrl+Shift+F lay out, F1 explain the mistake picked in
  the list, Esc or Ctrl+Q close. In the editor Tab indents (4 spaces) and
  Ctrl+Tab moves to the list; Enter on a mistake goes to it.
- Changes that aren't saved are never lost without asking (closing,
  opening another file, starting a new one).
- Files it won't open, with a message: more than 1 MB, not UTF-8, with a
  zero byte, or with a line longer than 10,000 letters (each would make
  the editor slow or the saved file different from what was opened).
- Permissions: `files.documents`. Programs are opened and saved through
  the file chooser, which runs inside the app, so installed it can reach
  your Documents folder and its own folders.
- **Run, installed:** NexaOS lets an installed app run only programs from
  the system's own folders, never files it has written (`nexa-run`'s
  Landlock rules give no folder that is both writable and runnable). So
  in an installed Code Studio, Run builds the program and then shows
  nexac's message that it can't be started here; checking, laying out and
  explaining work. From a terminal, `nexac run FILE.nx` runs it. Run works
  fully when Code Studio isn't confined (started from `build/`).
- Tests: `tests/*.steps` are driven with real key presses by
  `nexalang/tests/ui/drive.py`, and run by `cd nexalang && cargo test`
  three ways (leak counter, AddressSanitizer + UBSan, Valgrind); `*.out` is
  the exact output expected (the window's widgets and the text of the
  editor and output, printed with Ctrl+Shift+F12 in tests, and the files
  it saved).
