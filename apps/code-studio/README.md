# Code Studio

Write NexaLang programs. Code Studio shows mistakes while you type (a red
line under them, and a list with a hint for each), explains a mistake in
plain words, lays the code out the standard way, and runs the program
with its output in the window.

Written in NexaLang (`nexalang/NEXALANG.md`). The editor is `editor.new`
(GtkSourceView 4, with NexaLang's colours made from the theme), and
everything about the language comes from `nexac`, started with
`ui.start_program` (an argument list, never a shell): `nexac check --json
--stdin` for mistakes, `nexac fmt --stdin`, `nexac explain TOPIC`, and
`nexac build` to run. All of these are new in NexaLang 0.8.

- Build: `nexa-pack build` (needs `nexac` on the PATH)
- Keys: Ctrl+N new program, Ctrl+O open, Ctrl+S save, Ctrl+Shift+S save
  as, F5 or Ctrl+Return run (it's saved first), Shift+F5 stop,
  Ctrl+Shift+F lay the code out (Ctrl+Z undoes it), F1 explain the
  selected mistake, F8 next mistake, Esc or Ctrl+Q close (it asks first
  when there are changes that aren't saved). In the editor Tab indents and
  Ctrl+Tab moves on; in the list of mistakes Enter goes to the mistake.
- Files: only `.nx` files up to 2 MB, UTF-8, with no zero bytes; anything
  else is refused with a message and the open file stays. The NexaLang
  colours need `nexalang.lang` installed (NexaOS has it:
  `scripts/stage-text-editor.sh`); without it the text is plain.
- Permissions: `files.documents`, so the file chooser can open and save
  programs in your Documents folder (it runs inside the app, so an
  installed app can only use the folders its permissions name). A file
  given on the command line (`code-studio app.nx`) can be read; saving
  it somewhere else is Save as.
- **Run, installed**: Code Studio builds the program in
  `~/.cache/code-studio/run`, but NexaOS's app confinement (`nexa-run`,
  Landlock) doesn't let an app start a program from a folder it can write
  to. So installed, Run says so and the program can be run with
  `nexac run app.nx` in a terminal; checking, layout and explanations work.
  Changing this needs a decision about `nexa-run` (see
  `nexalang/NEXALANG.md`, known limits). Building also needs a C compiler
  and GTK's development files (`apk add gcc musl-dev pkgconf
  gtk+3.0-dev`), which NexaOS doesn't ship; nexac says so when they're
  missing.
- Tests: `tests/*.steps` are driven with real key presses by
  `nexalang/tests/ui/drive.py` and run by `cd nexalang && cargo test`
  three ways (leak counter, AddressSanitizer + UBSan, Valgrind); `*.out` is
  the exact output expected (the window's widgets, printed with
  Ctrl+Shift+F12 in tests, and the files it saved).
