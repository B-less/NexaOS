# NexaLang: the programming language of NexaOS

Started 2026-11-16. Current version **0.9** (compiler `nexac`, in `nexalang/`).

NexaLang is a small, safe, readable language for writing NexaOS programs.
The compiler `nexac` (Rust, no dependencies) turns a `.nx` file into C11
and builds it with **exactly the hardened flags WRITING_NEXA_APPS.md
requires** (`-Wall -Wextra -Werror -fstack-protector-strong
-D_FORTIFY_SOURCE=2 -fPIE -pie -Wl,-z,relro,-z,now`). So a NexaLang
program is, as far as the app rules go, a hardened C program, without
the author being able to write unsafe C.

```nx
use files
use json

fn load_name(path: str) -> !str {
    let config = json.parse(files.read(path)?)?
    match json.get(config, "name") {
        some(v) => { return ok(unwrap_or(json.as_text(v), "?")) }
        none => { return err("{path} has no \"name\"") }
    }
}

fn main() -> !void {
    let dir = files.config_dir("hello-app")?
    files.make_dir(dir)?
    let path = files.join_path(dir, "config.json")
    if not files.exists(path) {
        files.write(path, "\{\"name\": \"Bless\"}")?
    }
    print("Hello, {load_name(path)?}!")
    return ok()
}
```

A window, with NexaOS's look (Hexa) built in (0.4):

```nx
use ui

fn main() -> !void {
    let page = ui.start("hello-app", "Hello")?
    let name = ui.entry(page, "Your name")
    let greet = ui.primary_button(page, "Greet")
    let answer = ui.label(page, "")
    while true {
        match ui.wait() {
            Clicked(id) => {
                if id == greet {
                    ui.set_text(answer, "Hello, {ui.get_text(name)}.")
                }
            }
            Submitted(_, text) => { ui.set_text(answer, "Hello, {text}.") }
            Closed => { return ok() }
            _ => { }
        }
    }
    return ok()
}
```

An app for NexaOS, packaged (0.5):

```sh
nexa-pack new my-app --lang nexa    # nexa.toml, src/main.nx, icon.svg
cd my-app && nexa-pack build        # dist/my-app-0.1.0.nexa (built by nexac)
nexa-pack verify dist/my-app-0.1.0.nexa
```

`apps/focus-timer` is a complete NexaOS app written in NexaLang.

```sh
nexac run hello.nx                  # build and run
nexac run --check-memory hello.nx   # also report memory never freed
nexac build hello.nx -o hello       # native program
nexac check hello.nx                # mistakes only
nexac emit-c hello.nx               # see the generated C
```

Tools for writing it (0.6):

```sh
nexac fmt hello.nx                  # lay it out the one standard way
nexac check --json hello.nx         # mistakes as JSON, for editors
nexac explain                       # what each kind of mistake means
nexac explain results               #   one of them, with an example
nexac lsp                           # a language server, for editors
```

Colours for NexaOS's text editor and VS Code, and how to use the language
server in other editors: `editors/README.md`.

Windows you can change, and files the person picks (0.7):

```nx
let notes = ui.column(page)
ui.clear(notes)                                  // empty it, to fill it again
let code = ui.code(page, "fn main() \{ \}")       // read-only code that scrolls
match ui.save_file("Save the notes", "notes.txt") {
    some(path) => { files.write(path, ui.get_text(code))? }
    none => { }                                  // they cancelled
}
ui.to_clipboard("copied")
```

`apps/window-designer` builds a window from parts, shows it, and writes
the NexaLang program for it (a NexaOS app written in NexaLang).

A code editor, and other programs started from a window (0.8):

```nx
use ui
use editor

let code = editor.add(page)                       // GtkSourceView, NexaLang's colours
let out = ui.code(page, "")
let n = ui.start_program("", ["nexac", "check", "--json", "--stdin", "app.nx"], ui.get_text(code))?
match ui.wait() {
    Edited(id) => { }                             // the person typed in the editor
    Output(id, text) => { ui.append(out, text) }  // what the program printed
    Finished(id, code) => { }                     // its exit code
    _ => { }
}
editor.mark(code, 3, 7, 1)                        // underline a mistake
```

`apps/code-studio` is a NexaLang code editor written in NexaLang: mistakes
as you type, Lay out, Explain, and Run with Stop.

Writing NexaLang on Microsoft Windows (0.9), in VS Code:

```sh
sh editors/windows/build.sh     # dist/nexac.exe and dist/nexalang-0.9.1.vsix
```

Put `nexac.exe` in a folder of `PATH` on Windows, install the `.vsix` in VS
Code (Extensions, `...`, "Install from VSIX..."), and open a `.nx` file:
colours, mistakes as you type, layout on save, and an explanation of each
mistake. On Windows `nexac` checks, lays out and explains programs;
building and running them happens on NexaOS (see the decisions).
`editors/vscode/README.md` says more.

---

## 1. Design decisions (and why)

Each choice was made to lower the error rate: for the person writing code,
and for the compiler itself.

| Decision | Why | Rejected alternative |
|---|---|---|
| **Compile to C, then gcc/clang** | Fits the app rules (C with hardened flags) with no new runtime; gets gcc's optimiser and every CPU Alpine supports; GTK and GStreamer are C, so interop is direct later | Own VM/interpreter (slow, a second runtime to secure); LLVM (huge dependency) |
| **Compiler in Rust, std only** | AGENTS.md: OS work is Rust/C; no crates means offline, reproducible builds | Python (not allowed for system tools) |
| **Statements end at the newline** (Go's rule) | No `;` noise, and the rule is proven; inside `( )` and `[ ]` lines can be split freely | Significant indentation; mandatory `;` |
| **`let` can't change, `var` can** | Most values never change; the compiler catches accidental changes. Parameters are `let` | Everything mutable |
| **No shadowing** | A common source of "wrong variable" bugs | Rust-style shadowing |
| **Names: values lowercase, types Capitalised** (0.2) | `Shape.Circle` is then always a variant and `if x {` never a struct literal, with no guessing | Context-dependent parsing |
| **No implicit conversions** (`int + float` is an error) | Silent conversions lose precision unnoticed | C's promotions |
| **Every `int` operation checked**; **every list access bounds-checked** | In C these are undefined behaviour, the root of many security bugs | Wrapping arithmetic, unchecked indexing |
| **Strict left-to-right evaluation** | C leaves the order unspecified; the generator stores each value in a temporary in source order | Inheriting C's order |
| **No null: `?T` instead** (0.2) | A value that might be missing has a different type, and `match` makes you handle `none`. Null-pointer crashes can't be written | Null references |
| **`match` must cover every case** (0.2) | Adding a variant later shows every place that needs updating | Silent fall-through |
| **Reference counting, not a garbage collector** (0.2) | Memory is freed at a predictable moment (no pauses, important for smooth UI); a ~400-line runtime that can be checked by reading; the classic weakness (cycles) can't occur because no type can contain itself | Tracing GC (pauses, complex); manual free (unsafe); Rust-style borrow checker (much harder to learn and to build) |
| **Everything is a value; lists copy-on-write** (0.2) | `var b = a` then changing `b` never changes `a`, for every type. One rule, no aliasing surprises. Copying is free until one copy is changed | Shared lists (0.1's rule, the one that surprised people) |
| **Errors are values: `!T`** (0.3) | A function that can fail says so in its type, and the caller must handle the error (`match`, `unwrap_or`) or pass it on with `?`. Nothing is thrown past code that didn't expect it | Exceptions (invisible control flow); C-style error codes (easy to ignore) |
| **Modules are files; everything private unless `pub`** (0.3) | `use helpers` loads `helpers.nx` next to the file. Calls name their module (`files.read`), so it's always clear where a function comes from. Circles of modules are refused | Header files; global namespaces |
| **The standard library is written in NexaLang** (0.3) | `json`, `broker`, `files` and `time` get the language's checks (bounds, overflow, memory) for free; only the few calls into the OS are C (`__files_read` etc., callable only from standard modules) | A C standard library (every parser bug a memory bug) |
| **Every `str` is valid UTF-8** (0.3) | Files are checked when read; input and arguments have bad bytes replaced with `�`. Text functions can rely on it, and nothing can cut a letter in half | Bytes with an encoding "usually" |
| **File writes are all-or-nothing** (0.3) | `files.write` writes a temporary file, flushes it to disk and renames it over the old one, so a crash or full disk never leaves a half-written config | Writing in place |
| **Errors explain and suggest** | People coming from C, Python or JS type those by habit | Bare error codes |
| **Windows: the program asks for the next event** (`match ui.wait() { Clicked(id) => ... }`) (0.4) | NexaLang has no closures or function values, and callbacks are where GUI code goes wrong most: code running inside a GTK signal while other code is half-way through changing the same data. With one loop the program drives, GTK runs only inside `ui.wait()`; signal handlers only add an event to a queue. No re-entrancy, no hidden control flow, and `match` makes the program handle every kind of event (adding one later shows every place to update) | Callbacks (needs function values; re-entrancy; ownership of captured values across C); a declarative tree that's re-diffed (far more code to get wrong) |
| **Widgets are numbers, checked on every use** (0.4) | Keeps the API to plain functions and `int`s, which a beginner can follow. A number that isn't a widget, or a widget used for something it can't do (`set_items` on a text), stops the program with a message naming the program's own file and line, like a list index outside the list | Typed handles (a `Widget` type) would catch some of these before running; they're planned once the language has a way to define distinct number types without boilerplate |
| **Hexa can't be left out** (0.4) | The runtime ships WRITING_NEXA_APPS.md §4: the `@nexa_*` colours with the fallback block, following theme changes live, Inter and JetBrains Mono, 6/10/16 px corners, 4-32 px spacing, the 160 ms ease-out. A **second primary button stops the program**; a **list can't be made without its empty-state text**; Esc and Ctrl+Q always close. A rule that's built in can't be forgotten | Styling left to each app (each would have to get it right) |
| **Only the person's actions are events** (0.4) | `set_text`, `set_checked`, `set_items`, `select` block their own signals, so a program can't loop by reacting to its own change (a classic GUI bug) | GTK's default (every change fires) |
| **GTK doesn't change the program's locale** (0.4) | `gtk_init` would call `setlocale`, and in German or French `print(1.5)` would start writing `1,5`, breaking JSON and files. `ui.start` turns that off | GTK's default |
| **Waiting for the broker doesn't freeze the window** (0.4) | The socket request runs on a helper thread that only touches plain C buffers (no NexaLang values, no GTK); the window keeps drawing (but takes no clicks) until the answer is back. NexaLang code itself stays single-threaded | Blocking the GTK thread (a frozen window during a 2-minute install); threads in NexaLang (needs atomic counts and message passing first) |
| **A mistake inside a standard module is reported where the program called it** (0.4) | `ui.set_text(99, "x")` says `in app.nx at line 12`, not a line of `std/ui.nx` the author never wrote | Showing the module's own line |
| **GTK only for programs that `use ui`** (0.4) | Command-line programs stay small and start instantly; `nexac` adds `pkg-config gtk+-3.0`'s flags (as separate arguments, no shell) after every hardened flag | Linking GTK always |
| **The broker's answer has kinds** (`Done`, `NeedsPassword`, `Unverified`) (0.4) | The real protocol (nexa-brokerd) has three kinds of "not done" that need different handling; an `enum` makes the program handle each. "too many wrong passwords" and plain refusals are `err`, since asking again can't help. `broker.call` keeps the common case one line | Returning the raw JSON reply (0.3: every program had to know the protocol) |
| **NexaLang apps are built through nexa.toml's ordinary `[build] command`** (`nexac build src/main.nx -o build/<id>`) (0.5) | No new `nexa.toml` field (WRITING_NEXA_APPS.md: don't invent fields), so the package format, `nexa-pack verify` and the installer's checks don't change at all, and older `nexa-pack`s build these projects too. The hardened flags stay in one place (`nexac`) | A `[build] nexa = "src/main.nx"` field, or `nexa-pack` calling the compiler as a library: a second place that has to get the flags right, and a field old tools refuse |
| **`nexac build` makes the output's folder** (0.5) | A new project has no `build/` yet; failing there would be every beginner's first error | Making each template add `mkdir -p` (a shell step) |
| **Programs are reproducible by construction** (0.5) | Packages must be byte-identical when rebuilt. The generated C has no time or random names, and the C file is named after the output (`app.nexa.c`), not a random temporary file, so even the symbol table is the same. Tested by building twice in different folders | Normalising the binary afterwards (more code that can be wrong) |
| **`files.size(path)` to check before reading** (0.5) | Every file is untrusted: an app can refuse a file that's far too big without reading up to 256 MiB first. It comes back through the runtime's existing `!str` (one C function, no new C result type for the code generator) and is turned into an `int` in NexaLang | A size argument on every `files.read` (each call would need a number); a new `!int` C type in the runtime (more generated code to keep right) |
| **`ui.display` for one big number** (0.5) | Hexa's display size (22 pt) in JetBrains Mono, so a timer or a total doesn't jitter as it changes. Programs still can't set font sizes, so the type scale can't be broken | Letting programs choose font sizes |
| **The sample app needs no permissions** (`apps/focus-timer`) (0.5) | It shows everything an app needs (a window with Hexa, its own XDG data, hostile files, an empty state, shortcuts, packaging, running confined) without a file chooser, which `ui` doesn't have yet. An app that opened the person's files would have had to ask for a whole `files.*` folder instead | A notes or viewer app (needs a file chooser first, or broad permissions) |
| **A saved file that can't be read is never written over silently** (0.5) | The person may want it back; the app says what happened, keeps working, and only "Clear history" (with a question first) replaces it | Starting fresh and overwriting (silent data loss) |
| **`nexa-pack new --lang nexa` checks the id and escapes the name** (0.5) | A name like `My "App" {x}` would otherwise break `nexa.toml` or the program (`{` starts a value in NexaLang text). Ids that nexa.toml would refuse are refused before any files are made | Finding out at the first build |
| **`nexac fmt` works on the tokens, not on the syntax tree** (0.6) | It never adds, removes or reorders a token; it only changes spaces, indentation and empty lines. So it can't change what a program means, and that's checkable: the generated C must be the same (it is, for every program in the repository and every fuzzed one). Numbers (`0xff`, `1_000`), strings and the code inside `{...}` are copied exactly | Printing the syntax tree back out (every literal's spelling and every comment would have to be carried through the tree: many ways to lose a comment or turn `0xff` into `255`) |
| **`nexac fmt` never joins or splits lines** (0.6) | A line break ends a statement, so moving one can change the program; keeping every break where it is (empty lines aside) keeps the meaning. Long lines stay long | Wrapping at a width (each wrap would need its own proof that the meaning didn't change) |
| **`nexac fmt` refuses a file that doesn't parse** (0.6) | It says what's wrong and leaves the file alone; type mistakes don't stop it. Editors show the mistake instead | Formatting broken code as best it can (guessing at a half-written program can make it worse) |
| **Comments after code line up, two spaces after the longest code of the group** (0.6) | Keeps the habit the standard library and Focus Timer already had (aligned notes beside fields and variables), with one rule instead of each person's spacing | One space (destroyed every aligned table of notes) |
| **Every mistake has a topic, found from its message** (0.6) | `nexac explain TOPIC` and the JSON's `topic` come from one list of phrases in `explain.rs`. A test reads every message written in the compiler's source and fails if one has no topic, and the fuzzer checks the messages as they're really shown, so a new message can't be forgotten. Topic names are words (`let-var`, `maybe`), which a beginner can read and type | A number at each of ~130 places that make a message (each a place to pick the wrong one; `E0308` means nothing to someone learning) |
| **Explanations live in `nexac`, each with a refused and a fixed program** (0.6) | They work offline, always match the compiler of the same version, and a test checks that each "wrong" program is refused with its own topic and each "right" one is accepted (and formatted), so the examples can't go stale | Explanations on a website or in a separate document (drift from the compiler) |
| **`check --json` counts columns in characters; the language server converts to UTF-16** (0.6) | The JSON matches what `nexac check` prints and what a person sees; LSP's UTF-16 rule (an emoji is 2) is applied in one function, tested with emoji and accents | UTF-16 everywhere (wrong for everything that isn't LSP); bytes (wrong in any line with an accent) |
| **The language server is `nexac lsp`, using the same code as the command line** (0.6) | One program, so the editor can never disagree with the compiler. It does little: whole-text updates, diagnostics, formatting, hover explanations; one thread, nothing kept but the open texts. Hostile input (64 MiB / 8 KiB limits, bad JSON, bad UTF-8, wrong types) gets an answer, never a crash; a compiler panic would be caught and shown as one mistake | A separate server program (two versions to keep in step); incremental updates (range arithmetic is a classic source of editor bugs); completion and go-to-definition (they need analysis of half-written programs; later) |
| **One expected colouring for both grammars** (0.6) | The GtkSourceView file and the VS Code (TextMate) grammar are separate files, so they could drift; a test colours one sample with both (GtkSourceView itself, and VS Code's own engine `vscode-textmate`) and compares each with the same expected letters | Testing only that the files load |
| **`nexac` has its own small JSON reader and writer** (0.6) | No crates (the compiler stays std-only and builds offline); 300 lines, strict, 64 levels deep at most, tested against Python's `json` by the fuzzer and with every prefix and flipped byte of a message | A JSON crate (the first dependency); hand-building JSON strings (escaping mistakes) |
| **Nexa Mind isn't wired to `nexac explain` yet** (0.6) | Nexa Mind's tools (`nexa-tools`, docs/NEXA_MIND.md) read and change files; none runs programs, and adding one that runs a compiler is a new permission to design, not a side effect of a compiler release. What 0.6 gives it is `nexac explain --json`: offline, plain-language text (title, summary, explanation, examples) written for people and small models. The NexaLang code editor (0.8) shows it directly | Inventing a Nexa Mind interface it doesn't have |
| **Widgets can be removed: `ui.remove(id)`, `ui.clear(column or row)`** (0.7) | Window Designer rebuilds its preview after every change, and any app that shows things that come and go needs the same. A removed widget's number is never used again: using it stops the program with "widget 7 was removed", instead of silently meaning some newer widget (the classic stale-handle bug). Its shortcut, its events still waiting in the queue and, for the main button, the "one primary" place go with it | Reusing numbers (a kept number would quietly point at a different widget); only hiding (every rebuild would leave hidden widgets behind until the 100,000 limit) |
| **Widgets are kept in a table by number, and the 100,000 limit counts only widgets that exist** (0.7) | Numbers keep growing (to 2^31), so a program that rebuilds part of its window for hours never runs out; the memory is what's on screen, not everything ever made | An array indexed by number (it would grow with every widget ever made) |
| **The file chooser runs inside the program, and starts in Documents** (`ui.save_file`, `ui.open_file`) (0.7) | WRITING_NEXA_APPS.md §3: the person's files only through a file chooser. GTK's own chooser, with Hexa's colours, sized to the screen; it asks before replacing a file. Installed apps are confined by `nexa-run`, so it can save only where the app's permissions allow (an app that exports declares `files.documents`, never `files.home`); writing elsewhere fails with the app's own message. A name that isn't UTF-8 can't be NexaLang text, so the chooser says so and stays open | A desktop portal (NexaOS has no `xdg-desktop-portal`: a new D-Bus service to secure first); `files.home` for every app that saves (far too much); `GtkFileChooserNative` (without a portal it's the same dialog, with less say over its size and look); replacing bytes that aren't UTF-8 (it would write to a different file) |
| **Recent files are left as GTK keeps them** (0.7) | Switching GTK's recent files off makes it write an **empty** `recently-used.xbel` (tested): it would wipe the list every other app shares. Left on, GTK adds the chosen file, like every GTK app does; under `nexa-run` it can't, and GTK says so on stderr (two warnings) | Turning recent files off (data loss for other apps) |
| **`ui.code`: read-only text in JetBrains Mono, without ligatures** (0.7) | The designer shows the program it wrote; a beginner must see `->` and `==` as they type them, not as arrows. Read-only keeps it one-way: the program sets it, the person reads and copies it | An editable text area now (0.8's code editor brings an editing widget over GtkSourceView; two editing widgets would be two sets of rules to keep right) |
| **A list or code view makes the rows and columns around it take the free height** (0.7) | Before, a list inside a row inside a column stayed 120 px tall in a big window. Now what's meant to grow does, however deep it is; nothing else changes size | Asking programs to say what grows (more to get wrong) |
| **`ui.focus(list)` puts the keyboard on its selected row (or the first)** (0.7) | Focusing the list itself left the keyboard nowhere useful: Up and Down did nothing until Tab | Focusing the list widget (GTK's default) |
| **Tests can print a window's widgets: `NEXALANG_UI_TREE=1`, then Ctrl+Shift+F12** (0.7) | Window Designer must be shown to make the same window as the program it writes; printing both windows' visible widgets (kind, words, state, list rows, the status line) makes that an exact text comparison. Without the variable the key does nothing, so apps can't depend on it | Accessibility (AT-SPI: needs a session bus and much more code); screenshots alone (can't be compared exactly) |
| **Window Designer shows the preview in its own window, made with the same `ui` calls as the program it writes** (0.7) | What the preview shows is what the program will show; a test compares the two, widget by widget. It has no main button of its own, so the design's main button can be in the preview | A second window (`ui` has one window per program: every part of it would have to become per-window); running the exported program (needs `nexac` and a C compiler on the person's computer, and starting programs, which is 0.8's work) |
| **Designs are an outline: parts in order, each with a depth** (0.7) | NexaLang has no types that contain themselves (that's what keeps reference counting free of cycles), so a tree of parts can't be a NexaLang type. A list with depths can, it's easy to check (each part at most one level deeper than the one before, and only after a row or column), and moving a part is moving a run of the list. A file that breaks a rule is refused as a whole and left alone, never half-read | A tree in JSON (needs recursive types); a list of parts with parent numbers (numbers that can point anywhere, or in a circle) |
| **The written program names each part after its words and its kind** (`save_button`, `your_name_field`, `row1`) (0.7) | Readable for a beginner, and the ending (`_button`, `_field`, a number) means a name can never be a NexaLang word, a name the program already uses (`page`, `id`, `text`, `on`, `index`) or another part's name. Texts are escaped (`\` `"` `{` `}`), so words like `{Save}` stay words | Numbered names (`button3`: unreadable); names from the words alone (could be `if` or `page`) |
| **Window Designer needs only `files.documents`** (0.7) | Exporting saves where the person picks, and installed it can only be Documents; Copy (Ctrl+Shift+C) needs no permission at all. It explains this when a save elsewhere fails | No permissions (Export could only save in its own hidden folder); `files.home` |
| **The code editor is its own standard module, `editor`; only programs that `use editor` need GtkSourceView 4** (0.8) | The same rule as GTK for `ui` (0.4): a program says what it needs at the top, `nexac` adds `pkg-config gtksourceview-4`'s flags only then (argument lists, after every hardened flag), and every other window program, and its package, stays as it was (no new `depends`). The editor is still a widget in `ui`'s table: `get_text`, `set_text`, `focus`, `remove` work on it | Functions in `ui` (every window program would link GtkSourceView, and every package would need it); an editing widget over plain GtkTextView (no colours, undo or indentation: hundreds of lines to write and get right that GtkSourceView already has) |
| **NexaLang's colours and the Horizon fallback scheme are built into the program** (0.8) | `nexalang.lang` and a fallback copy of the theme's editor colours are compiled in and written to the app's own cache folder (`$XDG_CACHE_HOME/<id>/nexalang/`, only when they differ, all-or-nothing), searched before the system's. So an editor always colours with the language file of the compiler that built it, installed or not. The theme's own scheme (`nexa.xml`, which `nexa-theme` writes) wins when it's there, and is followed live; nexa-run now lets apps read it (read-only, like `gtk.css`) | Depending on `/usr/share/.../nexalang.lang` (an older or missing file colours wrongly); a GtkSourceView scheme with hard-coded colours only (WRITING_NEXA_APPS.md §4: follow the person's theme) |
| **`Edited(id)` carries no text, and only one waits at a time** (0.8) | Copying a whole file into every key press's event would be megabytes per second for a big file; the program asks with `ui.get_text` when it needs the text (Code Studio: 0.4 s after the last key). However fast the person types or pastes, the queue holds at most one `Edited` per editor. Like every event, only the person's changes count (`set_text`, `replace_text` never send one) | `Changed(id, text)` as text fields do (text copied on every key press) |
| **Other programs are started from the window's own loop: `ui.start_program(folder, args, input) -> !int`, then `Output(n, text)` and `Finished(n, code)` events** (0.8) | Code Studio must run `nexac` without freezing the window and must be able to stop a program that never ends; the event loop programs already have is where that fits (no threads, no callbacks). An argument list, never a shell (WRITING_NEXA_APPS.md §1); the first word is looked up where the system keeps programs. Output and errors come together, in the order printed, in whole lines (a line longer than 64 KiB in pieces, never cutting a letter in half; zero bytes as U+FFFD), so text is always valid NexaLang text. After 8 MB the rest is read and dropped, with a note, so a program printing forever can't fill memory. Each program gets its own process group: `stop_program` sends SIGTERM to it and all it started, then SIGKILL 2 s later; stopping one that has just finished does nothing (a race the person can't avoid). At most 8 at once. When the window's program ends (or is stopped by a signal), the ones still running are stopped | `popen`/`system` (a shell); a blocking `run` (no Stop, frozen window); stdout and stderr as two event kinds (order lost); killing only the program started (`nexac run`'s child would keep running) |
| **Input goes through a socket written with `MSG_NOSIGNAL`** (0.8) | A program that stops reading its input early must not kill the window with SIGPIPE, and ignoring SIGPIPE for the whole process would change it for every program started too | `signal(SIGPIPE, SIG_IGN)` (inherited by every program it starts); blocking writes (a deadlock when the program writes before it reads) |
| **`start_program` comes back as an `int`, with the reason in a second call, turned into `!int` in `ui.nx`** (0.8) | The same choice as `files.size` (0.5): no new C result type for the code generator to keep right | An `nx_R_i` in the runtime |
| **New kinds of event are added to `UiEvent`, so `match`es that list every kind must list them too** (0.8) | 0.4 chose exhaustive `match` so that a new kind of event shows every place to update: the compiler names `Edited`, `Output` and `Finished` in each such `match` (Focus Timer, Window Designer and `examples/ui/tasks.nx` got three lines each; programs with `_ => { }` didn't change) | A second event type and a second `wait` (two loops to keep in step) |
| **`nexac run` deletes the program it built as soon as it has started, and asks it for its output line by line** (0.8) | A Stop button stops `nexac` and the program together; with the file already gone nothing is left in `/tmp`. Output through a pipe is normally sent 4 KB at a time, so a program would look silent in an editor until it ends; `NEXALANG_LINE_OUTPUT=1` (set by `nexac run` only) makes it line by line. `nexac build` programs are unchanged | Cleaning up after a signal (Rust's std has no signal handlers); a pseudo-terminal (much more code) |
| **Code Studio uses `nexac check --json`, `fmt --stdin`, `explain --json` and `run`, not the language server** (0.8) | Each is one short program run (a few ms) with the same answers as the command line, already tested by 0.6's fuzzing; there's no session to keep in step. The language server needs a long-running process and JSON-RPC framing written in NexaLang: more to go wrong for no new information. VS Code (0.9) uses the language server | A language server client in NexaLang |
| **Code Studio needs only `files.documents`, and installed it can't Run** (0.8) | Programs are opened and saved through the chooser, which runs inside the app, so it can only reach what the app may: Documents. `nexa-run` (Landlock) gives an app no folder it can both write and run programs from, so a program it builds can't be started; checking, laying out and explaining work, and Run shows `nexac`'s message saying so. That's NexaOS's protection against apps running what they download or write; loosening it for one app is a decision for NexaOS's author (a "developer tools" permission), not a side effect of an editor | `files.home`; adding an executable folder to nexa-run for this app; running the built program through the dynamic loader (a way around Landlock's check: never) |
| **Files Code Studio won't open: over 1 MB, not UTF-8, with a zero byte, or a line over 10,000 bytes** (0.8) | Measured: GtkSourceView takes ~1.2 s to show 2 MB and a 1 MB line makes every key press slow; a zero byte can't be shown and would be saved as U+FFFD (a silent change); bytes that aren't UTF-8 can't be NexaLang text. Each is refused with a message that says why | Opening anything (slow, or a different file saved) |
| **Code views wrap long lines; an editor takes the free height and the lists and code views beside it keep theirs; both sit in a card** (0.8) | WRITING_NEXA_APPS.md §4: text wraps, never goes off the side (an explanation was cut off at 560 px). In a code editor's column the editor is what needs the room (the list and output are 80 and 112 px at least, so everything fits in 640 px). GTK 3 doesn't clip a scrolled view to rounded corners; a padded card around it does | Horizontal scrolling for read-only text; sharing the height equally (a half-size editor) |
| **A button can have more than one shortcut** (0.8) | Run is Ctrl+Return and F5; the tooltip lists both | One key per action |
| **On Windows, `nexac` checks, lays out, explains and is a language server; `build` and `run` say programs are built on NexaOS** (0.9) | The C a program becomes uses POSIX (Unix sockets for the broker, signals, process groups, `fsync`/`rename` file writes, `getrlimit`) and GTK's Unix parts (`glib-unix.h`, `waitpid`). A Windows version of that runtime is a second runtime to write and secure, and it couldn't be run here at all (no Windows, no MSYS2 GTK). Editing is what Windows needs for now: `nexac check` there gives exactly the mistakes NexaOS gives (tested), and the message names the way to run the program (copy it to NexaOS, `nexac run`). The program is checked first, so its mistakes are still shown | Porting the runtime to MSYS2 MinGW + GTK 3 (large, and untestable here: it would ship unrun); building C on Windows that then fails (a confusing C compiler error instead of one clear sentence) |
| **`nexac.exe` is the same source, cross-built with mingw-w64 (`x86_64-pc-windows-gnu`)** (0.9) | Only three places differ by platform (the refusal above, the "stopped by signal" message, and how a `file:` URI becomes a path), so it can't drift from the Linux compiler: a test compares the two on every `.nx` file in the repository and on fuzzed programs, byte for byte. It needs only Windows' own DLLs (no `libgcc`, no `libwinpthread`), so one file is all there is to copy. `editors/windows/build.sh` removes this computer's paths from it | The MSVC target (needs Microsoft's SDK and linker: can't be built on Linux); a separate Windows tool (two compilers to keep in step) |
| **`nexac fmt` keeps the file's line ends** (`\r\n` stays `\r\n`) (0.9) | On Windows, Notepad, VS Code and git (`core.autocrlf`) write `\r\n`; 0.6 turned them into `\n`, so `nexac fmt --check` would call every such file unformatted and `fmt` would rewrite every line. The first line end decides (a file with both gets the first one's everywhere). Only line ends change: strings and comments can't hold a line break, so the meaning can't (still tested on every program) | Always `\n` (0.6: every Windows file "unformatted"); always the computer's own (the same file formatted differently on two computers) |
| **A module's file must be spelled as in `use`, capitals too, on every computer** (0.9) | Windows (and macOS) would find `Helpers.nx` for `use helpers`; NexaOS wouldn't, so a program checked on Windows would fail on NexaOS. `nexac` looks at the real file names and says "the file here is called Helpers.nx, and capitals count", on Linux too (where before it said only "can't find module"). For the same reason a `Json.nx` beside `use json` is no longer "also a json.nx here" on Windows | Trusting the file system (works on one computer, not the other) |
| **The VS Code extension talks to `nexac lsp` through VS Code's own language client** (`vscode-languageclient`) (0.9) | Mistakes, layout and hover then come from the language server 0.6 built and fuzzed, with its UTF-16 positions already tested (emoji, accents); the client is Microsoft's, used by most language extensions, so framing, restarts and document syncing aren't new code. The extension itself is ~330 lines of JavaScript: finding `nexac`, the "Explain" command and quick fix, and the settings | Its own LSP client (framing and syncing written again); running `check --json` on every key press (each keystroke a new process, and characters → UTF-16 converted again in JavaScript, a known source of off-by-one bugs) |
| **The extension starts `nexac` only by its full path, with an argument list, never a shell** (0.9) | Started by name, Windows looks in the current folder first, so a `nexac.exe` inside a downloaded project would run. The extension finds `nexac` itself: the `nexalang.path` setting (it must be a full path, and it has `machine` scope: a folder's `.vscode/settings.json` can't set it), else the first `nexac.exe` in a folder of `PATH` written as a full path (`.` and empty entries are skipped); it starts it in the home folder. Only `.exe` (a `.bat` or `.cmd` needs a shell). So it can be trusted in an untrusted folder: `nexac` only reads `.nx` files there | Starting `"nexac"` and letting the system search (the open folder could choose the program) |
| **`nexac lsp --stdio` is accepted, and the extension gives no transport** (0.9) | `vscode-languageclient` adds `--stdio` to the arguments when told the transport is stdio (found by reading its code before the first run); 0.6's `nexac lsp` refused any argument. Now it accepts that one (many editors' clients add it), and the extension says nothing about the transport, which means stdin/stdout without the extra argument. Both are tested | Leaving it (the server wouldn't start in VS Code) |
| **The extension is plain JavaScript, with no build step** (0.9) | What runs in VS Code is exactly the file that's reviewed and tested; no TypeScript compiler, bundler or source maps between them. Its one dependency (`vscode-languageclient`) is pinned with `package-lock.json` and packed into the `.vsix` by `vsce` | TypeScript + a bundler (a build step whose output nobody reads) |
| **Binaries aren't committed: `editors/windows/build.sh` makes `dist/nexac.exe` and the `.vsix`** (0.9) | The repository has no built programs in it (packages are built by scripts); a committed `.exe` couldn't be checked against its source. The script builds the same bytes each time (tested: two folders, the same `nexac.exe`) | Committing `nexac.exe` and the `.vsix` |
| **Passwords are asked for by `ui`, checked by the broker** (0.4) | `ui.broker_call` shows a password dialog when the broker asks, three tries, the way `nexa_common/broker.py` does; the password goes only into the request to the broker. Apps never check or keep it | Apps asking with their own dialogs |

## 2. The language (v0.9: no changes since 0.5; `ui` grew in 0.7 and 0.8; `editor` is 0.8's)

**Types:** `int` (64-bit, checked), `float` (64-bit), `bool`, `str`
(immutable UTF-8 text), `[T]` (growable list), `?T` (a `T` or `none`),
`!T` (a `T` or an error message; `!void` for "worked or failed"),
`struct`s and `enum`s.

**Values:** `42`, `1_000_000`, `0xff`, `3.14`, `2e-3`, `true`, `"text"`,
`[1, 2, 3]`, `Point { x: 1, y: 2 }`, `Shape.Circle(2.0)`, `Light.Red`,
`some(5)`, `none`, `ok(5)`, `ok()`, `err("message")`.

**Strings:** `"Hello {name}, you have {len(items)} items"` puts any `int`,
`float`, `bool` or `str` value in the text, including calls with their
own strings: `"{join(names, ", ")}"` (up to 8 levels). Escapes:
`\n \t \r \0 \\ \" \{ \}`. `+` joins; `==`, `<` compare. Floats print the
shortest form that reads back exactly (`1000.0`, `0.30000000000000004`,
`3e+20`).

**Declarations:**
```nx
struct Team {
    name: str
    members: [str]
}

enum Msg {
    Text(str)
    Move(int, int)
    Quit
}

fn greet(t: Team) -> str { ... }
```

**Statements:**
```nx
let x = 1                // fixed
var y: float = 0.0       // changeable
y += 2.5                 // = += -= *= /= %=
if a and not b { } else if c or d { } else { }
while cond { break / continue }
for i in 0..10 { }       // 0 to 9
for item in list { }     // loops over the list as it was when the loop started
match msg {
    Text(t) => { print(t) }
    Move(x, _) => { }    // `_` ignores a value
    _ => { }             // everything else
}
match maybe_value {
    some(v) => { }
    none => { }
}
match result {
    ok(v) => { }
    err(message) => { }
}
match answer {           // numbers and text: `_` is required
    "yes" => { }
    "no" => { }
    _ => { }
}
let n = parse_number(s)? // the value, or return the error/none to the caller
return value
```

**`?`** works on `!T` inside a function that returns a `!` type, and on
`?T` inside one that returns a `?` type. `fn main() -> !void` may use it
too: an error reaching the end of `main` is printed (`error: ...`) and the
program exits with status 1.

**Modules:**
```nx
use files            // a standard module
use shapes           // shapes.nx, next to this file

fn main() {
    let s = shapes.circle(2.0)     // functions are called through their module
    let t = Shape.Square(1.0)      // pub types are used by name
}
```
In `shapes.nx`, only items marked `pub` (`pub fn`, `pub struct`,
`pub enum`) can be used by other files. Type names must be different
across all the files of a program.

**Operators** (low to high): `or`, `and`, `not`, comparisons
(`== != < <= > >=`, not chainable; `==` also works on enums whose variants
hold nothing), `+ -`, `* / %`, unary `-`, then `f(x)`, `a[i]`, `a.b`.
Integer `/` rounds toward zero; `%` takes the sign of the left side.

**Built-ins:**

| Group | Functions |
|---|---|
| General | `print(x)`, `len(list or str)`, `to_str(x)`, `to_float(int)`, `to_int(float)`, `assert(cond)` |
| Lists | `push(list, x)`, `pop(list)` |
| Maybe | `some(x)`, `unwrap_or(maybe, default)` |
| Text | `slice(s, start, end)`, `find(s, part) -> ?int`, `contains`, `starts_with`, `ends_with`, `split(s, sep) -> [str]`, `join(list, sep)`, `trim`, `to_upper`, `to_lower`, `replace(s, from, to)`, `repeat(s, n)` |
| Reading | `parse_int(s) -> ?int`, `parse_float(s) -> ?float` (strict: no spaces, hex, `nan` or `inf`) |
| Input | `read_line() -> ?str` (`none` at end of input), `args() -> [str]` |
| Results | `ok(x)`, `ok()`, `err(message)`; `unwrap_or` works on `!T` too |
| Bytes | `byte_at(s, i) -> int`, `char_from_code(n) -> str`, `exit(code)` |

**Standard modules (0.3):**

| Module | Functions |
|---|---|
| `files` | `read(path) -> !str`, `read_lines -> ![str]`, `size(path) -> !int` (bytes; 0.5), `write(path, text) -> !void` (all-or-nothing), `append`, `exists`, `is_dir`, `list(dir) -> ![str]` (sorted names), `make_dir` (with parents), `remove` (a file or an *empty* folder; never contents), `join_path(a, b)`, `name(path)`, `home()`, `config_dir(id)`, `data_dir(id)`, `cache_dir(id)` (the app's XDG folders; ids follow nexa.toml's rules) |
| `time` | `now() -> int` (Unix seconds), `millis()` (for measuring), `sleep_ms(n)`, `format(t, "%Y-%m-%d")`, `format_utc`, `iso(t)` |
| `json` | `parse(text) -> !Json` (errors give line and column; 200 levels max), `to_text(j)`, `pretty(j)`, `get(j, key) -> ?Json`, `at(j, i)`, `as_text`, `as_num`, `as_int`, `as_bool`, `items`, `keys`, `obj(fields)`, `field(key, value)`, `set(j, key, value)`. Values: `Json.Null`, `Json.Bool(b)`, `Json.Num(n)`, `Json.Text(s)`, `Json.List(items)`, `Json.Obj(fields)` |
| `broker` (0.4) | `call(action, args: Json) -> !Json` (the action's result; refusals, a needed password and unverified packages are `err`), `ask(action, args, password: ?str) -> !BrokerReply` (`Done(result)`, `NeedsPassword(what, why)`, `Unverified(why)`; a plain refusal or "too many wrong passwords" is `err`), `call_timeout`, `ask_timeout`, `no_args()`. One request to `/run/nexa-broker.sock`; 30-minute limit |
| `ui` (0.4) | see below |
| `editor` (0.8) | a code editor for NexaLang in a window; see below |

**Windows: `ui` (0.4).** GTK 3 with Hexa built in. Every function that
makes a widget takes its parent (a column or a row) and returns the new
widget's number.

| Group | Functions |
|---|---|
| Window | `start(app_id, title) -> !int` (opens the window, returns its main column; `err` when there's no display; one window per program) |
| Layout | `column(parent)`, `row(parent)` (text fields and columns in a row share the free width) |
| Text | `label`, `heading`, `note` (small, quiet), `mono` (JetBrains Mono: numbers that update), `display` (one big number: JetBrains Mono, 22 pt; 0.5) — all `(parent, text) -> int`; long text wraps |
| Controls | `button(parent, text)`, `primary_button` (one per window, accent colour), `danger_button`, `entry(parent, hint)`, `check(parent, text, on)`, `list(parent, empty_text)` (scrolling rows of text) |
| Code (0.7) | `code(parent, text) -> int`: read-only text in JetBrains Mono that scrolls (no ligatures: `->` shows as typed); `set_text` and `get_text` work on it |
| Changing | `set_text(id, text)`, `get_text(id)`, `set_checked`, `is_checked`, `set_items(list, [str])` (clears the selection), `selected(list) -> ?int`, `select(list, i)`, `select_none`, `set_enabled`, `set_visible`, `focus` (a list: its selected row, or the first), `tooltip(id, text)`, `shortcut(button, "Ctrl+N")` (shown in the tooltip), `status(text)`, `show_error(text)` (the line at the bottom) |
| Removing (0.7) | `remove(id)` (the widget and what's in it), `clear(column or row)` (what's in it). Removed numbers are never used again; using one stops the program with a message |
| Files and clipboard (0.7) | `save_file(title, name) -> ?str` (the chooser, starting in Documents, `name` suggested; asks before replacing), `open_file(title, "*.nx") -> ?str` (`""` for every file); none when cancelled. `to_clipboard(text)` |
| Adding (0.8) | `append(code, text)`: text at the end of a code view, which then shows its end (a log, a program's output) |
| Programs (0.8) | `start_program(folder, args: [str], input) -> !int`: another program, started with an argument list (never a shell; the first word found where the system keeps programs; `folder` `""` for here). `input` is what it reads. Its output (and errors, in order) comes as `Output(n, text)` in whole lines, then `Finished(n, exit code)` (128 + the signal when stopped). `stop_program(n)`: SIGTERM to it and what it started, SIGKILL 2 s later; nothing when it has finished. At most 8 at once; 8 MB of output each (the rest is dropped, with a note); the ones still running are stopped when the window's program ends |
| Events | `wait() -> UiEvent`, `wait_ms(ms) -> UiEvent` (`Tick` when nothing happened) |
| Questions | `confirm(heading, message, yes, danger) -> bool`, `ask_password(what, why) -> ?str`, `broker_call(action, args) -> !Json` (the broker, asking for the password in the window when needed: three tries) |

```nx
pub enum UiEvent {
    Clicked(int)          // a button, or its shortcut
    Changed(int, str)     // the person edited a text field: its text
    Submitted(int, str)   // Enter in a text field
    Toggled(int, bool)    // a check box
    Selected(int, int)    // a list and its selected row (from 0)
    Activated(int, int)   // a list row double-clicked, or Enter on it
    Closed                // the close button, Esc or Ctrl+Q
    Tick                  // wait_ms: nothing happened in time
    Edited(int)           // the person changed a code editor's text (0.8)
    Output(int, str)      // a program from start_program printed this (0.8)
    Finished(int, int)    // that program ended: its exit code (0.8)
}
```

**The code editor: `editor` (0.8).** `use editor` (with `use ui`). Only
programs that use it need GtkSourceView 4 (NexaOS has it: `depends =
["gtk+3.0", "gtksourceview4"]`).

| Function | What it does |
|---|---|
| `editor.add(parent) -> int` | A NexaLang editor: colours (the language file built into `nexac`), the theme's editor colours (`nexa.xml`, followed live; the Horizon fallback otherwise), line numbers, 4-space indents, undo (Ctrl+Z, Ctrl+Shift+Z), JetBrains Mono without ligatures. Tab indents; Ctrl+Tab moves on. It takes the free height. `ui.get_text`, `ui.set_text` (new text, which can't be undone), `ui.focus`, `ui.set_enabled`, `ui.remove` work on it. The person's changes send `Edited(id)` (one waits at a time) |
| `editor.replace_text(id, text)` | All the text replaced as one change the person can undo; the cursor stays on its line (for `nexac fmt`'s result). Nothing happens when the text is the same |
| `editor.mark(id, line, column, length)` | Underline a mistake in the theme's `@nexa_crit` (lines and columns from 1, columns in letters, as `check --json` gives them). A place that isn't in the text any more is left alone; a column past the end of the line marks its last letter |
| `editor.clear_marks(id)` | Take every underline away |
| `editor.go_to(id, line, column)` | Put the cursor there (past the end: the nearest real place), show it, and put the keyboard there |

Keyboard: Tab moves between widgets (and out of a list; arrow keys move
inside it), Space and Enter press, Esc and Ctrl+Q close. `Closed` doesn't
close the window by itself: the program decides (returning from `main`
does). Windows are 560×640 to start and work down to 360 px wide. Lists
and code views take the free height, and so do the rows and columns they
are in (0.7).

For tests (0.7): with `NEXALANG_UI_TREE=1` in the environment,
Ctrl+Shift+F12 prints the window's visible widgets and the status line
(`tests/ui/drive.py` sets it; without it the key does nothing). With
`NEXALANG_UI_TREE=full` (0.8) it also prints the text of code views and
editors, line by line, and an editor's underlines, cursor and colours.

Code views (0.8) wrap long lines. A button can have more than one
shortcut (0.8); its tooltip lists them all.

JSON numbers are `float`, as in JavaScript: whole numbers beyond 2^53
are rounded, and numbers too big for a `float` are refused (JSON has no
infinity).

Text positions are byte offsets; `slice` refuses to cut a UTF-8 letter in
half. `to_upper`/`to_lower` change ASCII letters only (for now).

**Semantics to know:**
- **Everything is a value.** `var b = a` and passing to a function never
  lets one copy change the other, for structs, enums and lists alike.
- Changing a list (`push`, `pop`, `a[i] = ...`) needs the variable it's
  reached through to be `var`.
- A function that returns a value must return on every path (checked,
  including through `match`).
- `main` is `fn main()`: no parameters, returns nothing.
- A function that calls itself on every path before it can return is
  refused (it could never finish). Recursion that runs too deep anyway
  stops with "the program ran out of stack space" (0.4).

**Tools (0.6).**

| Command | What it does |
|---|---|
| `nexac fmt FILE.nx...` | Lays files out the standard way, in place (all-or-nothing, keeping the file's permissions; a link's target is replaced). `--check` only lists files that aren't laid out that way (exit 1); `--stdin` formats stdin to stdout. A file that doesn't parse is left alone, with its mistake shown |
| `nexac check --json [--stdin] FILE.nx` | One line of JSON: `ok`, and each mistake's `file`, `line`, `column` (characters, from 1), `length`, `message`, `hint`, `topic`, `summary`. `--stdin` checks unsaved text; the modules next to `FILE.nx` are still read from the disk. Format: `editors/README.md` |
| `nexac explain [--json] [TOPIC or "message"]` | With nothing: the 20 topics. With a topic (`let-var`) or a message nexac gave: what it means, how to fix it, a refused program and the fixed one |
| `nexac lsp` | A language server on stdin/stdout: mistakes while typing, formatting, an explanation when the pointer rests on a mistake |

The standard layout: 4 spaces per level (a line is one level deeper than
the line that opened the innermost unclosed bracket; a line starting with
a closing bracket lines up with its opener); one space between tokens,
except none inside brackets, before `,` `:` `)` `]`, around `.` and `..`,
before a call's `(` or an index's `[`, after a unary `-`, inside `?int`
and `!void`, and before the `?` that passes an error on; at most one empty
line in a row (none after an opening line or before a closing one);
comments after code lined up two spaces after the longest code of their
group. Line breaks stay where they are.

Topics: `syntax`, `strings`, `numbers`, `names`, `unknown`,
`already-defined`, `let-var`, `types`, `match`, `maybe`, `results`,
`returns`, `unused`, `modules`, `lists`, `main`, `calls`, `loops`,
`limits`, `files`.

## 3. How memory works (0.2)

Strings and lists carry a reference count. The code generator gives every
value exactly one owner:

- **Owned** values (variables, temporaries holding new values, list items,
  fields, variant contents) are released exactly once: at the end of the
  block that made them, or earlier when `break`, `continue` or `return`
  leaves the block.
- **Borrowed** values are views of something owned that lives longer: a
  parameter (parameters can't change, so the caller's value outlives the
  call), a list item read in an expression, a field of an owned value.
- Keeping a borrowed value (storing it in a variable, a list, a field, or
  returning it) adds a reference; keeping an owned temporary moves it.
- Changing a list first makes it private if it's shared (copy-on-write),
  including lists inside lists and inside structs (`grid[1][1] = 0`).
- `for x in list` loops over a private snapshot; `match v` holds its own
  reference, so a case can replace `v` safely.

**Across the C boundary (0.4).** Text and lists passed to `ui` are
borrowed: the runtime copies what GTK needs and keeps nothing. Text that
comes back (`get_text`, event text, a password) is a new NexaLang string
with one owner, freed like any other. GTK's own memory is GTK's;
`--check-memory` counts every NexaLang byte exactly, with or without a
window. A chosen file's path (0.7) is new text with one owner, like
`get_text`. 0.8: a program's output (`Output`) is new text with one owner,
copied out of the runtime's own buffers; the argument list and input given
to `start_program` are borrowed and copied (the input into a buffer the
runtime frees once it's written); a running program's record is freed
when it has finished and its `Finished` has been queued. Removing a widget (0.7) frees the runtime's record of it
(its tooltip, its shortcut, the text of its events still waiting) at
once; GTK frees the widget.

Text pieces from `slice`, `split` and `trim` share the original bytes
instead of copying them. Literal text is never counted or freed.

## 4. How the compiler is built

```
nexalang/
  src/lexer.rs     text -> tokens (newline rule, nested string interpolation)
  src/parser.rs    tokens -> syntax tree (stops at first error; depth limit 200)
  src/checker.rs   types, names, mutability, returns, match coverage,
                   types containing themselves; reports all errors
  src/codegen.rs   syntax tree -> C: left-to-right temporaries, ownership,
                   generated retain/release functions per type
  src/runtime.h    checked ints, counted strings, copy-on-write lists,
                   text functions, input, leak counter (~450 lines)
  src/runtime_ui.h windows (GTK 3): widgets, the event queue, Hexa's CSS,
                   theme following, dialogs (only in programs that use ui)
  src/main.rs      CLI and module loader (finds `use`d files, refuses circles);
                   runs cc with an argument list, never a shell
  src/diag.rs      a mistake with its place; `Report`: ready for text or JSON
  src/fmt.rs       `nexac fmt` (0.6): re-lays tokens and comments out
  src/explain.rs   `nexac explain` (0.6): topics, their phrases, examples
  src/json.rs      a small strict JSON reader/writer (0.6)
  src/lsp.rs       `nexac lsp` (0.6): the language server
  src/std/*.nx     the standard modules, built into nexac (editor.nx: 0.8)
  examples/*.nx    + .out expected output (+ .in input, .args arguments)
  tests/fail/*.nx  programs that must be rejected (64)
  tests/trap/*.nx  programs that must stop with a runtime error (11)
  tests/modules/   a three-file program, and 12 module mistakes
  tests/broker/    a stand-in broker (Python) and a client
  tests/json/      JSON round-trip reader + differential fuzz vs Python
  tests/stress/    2-million-round memory churn
  tests/ui/        window programs driven by real key presses on a
                   headless X server (drive.py: Xvfb + xdotool), *.steps
                   with exact *.out; tests/ui/trap: misuse (20)
  tests/fuzz/      the compiler fuzzer (fuzz.py)
  examples/ui/     window examples
  tests/tooling.rs fmt / check --json / explain / lsp tests (0.6)
  tests/highlight/ both colour grammars against one expected colouring
  editors/         gtksourceview/nexalang.lang (built into programs that use
                   `editor`, 0.8) and nexa-horizon.xml (their fallback
                   colours), vscode/ (grammar; the extension, 0.9:
                   extension.js, lib.js, package.json, test/), windows/
                   build.sh (nexac.exe and the .vsix, 0.9), README
  tests/windows/   nexac-wine.sh (runs nexac.exe under Wine);
                   differential.py: the Windows nexac against the Linux
                   one, on every .nx file and on fuzzed programs (0.9)
../apps/*/src/main.nx   NexaOS apps written in NexaLang (0.5): Focus Timer,
                 Window Designer (0.7), Code Studio (0.8); their tests/*.steps run with the
                 window tests, three ways; the programs Window Designer
                 exports in them are built and run too
../tools/nexa-pack      `nexa-pack new --lang nexa` (src/templates.rs)
```

All identifiers are prefixed in C (`nx_v_`, `nx_fn_`, `nx_S_`, `nx_E_`,
`nx_M_`, `nx_R_`, `nx_f_`), so a NexaLang name can never collide with C;
module functions get their module's name with its length
(`nx_fn_4json_parse`). A call from the program into a standard module
first records its location (`nx_user_loc`), so a runtime error inside the
module is reported at the program's line. String literals are emitted with octal escapes
only. A runtime error knows its file: the location passed to the runtime
is file number × 1,000,000 + line.

## 5. Testing done

**0.9 (2026-11-18)** — Ubuntu 24.04 here: gcc 13.3, mingw-w64 (gcc
13), Rust 1.97 (its Windows libraries built from Rust 1.97's own library
source: this machine couldn't download Rust's ready-made ones), Wine 9.0,
Node.js 22, Valgrind, GTK 3.24.41, GtkSourceView 4.8.4, Xvfb.
`cd nexalang && cargo test`: **68 tests, all pass** (29 unit, 26 end to
end, 13 in `tests/tooling.rs`). No earlier test was removed or weakened;
one expectation changed with `fmt`: a `\r\n` file now stays `\r\n`
(0.6's unit test expected `\n`; it now expects the line ends kept, and a
new test covers mixed and comment-ending lines). New: the extension's
tests run from `cargo test` (`the_vscode_extension_passes_its_tests`).

| Check | Result |
|---|---|
| Every earlier check (examples with gcc and clang, ASan + UBSan, Valgrind, window scenarios, Focus Timer's, Window Designer's and Code Studio's scenarios three ways, misuse, broker, modules, rejected programs, runtime errors, the 0.6 tools, reproducible builds) | all still pass |
| **Every 0.6 tool test against `nexac.exe` under Wine** (`NEXAC_TEST_WINDOWS=1 NEXAC_TEST_EXE=<a script running wine nexac.exe> cargo test --test tooling`): fmt never changes meaning on every program in the repository, fmt in place (all-or-nothing replace on Windows), `check --json`, explain's examples, whole language server sessions, modules | **all pass** (only the Unix file mode check is skipped: Windows has no modes) |
| **The Windows `nexac` against the Linux one** (`tests/windows/differential.py`): `check --json`, `emit-c`, `fmt --stdin` (also with `\r\n`) and `explain --json` on all 167 `.nx` files in the repository and apps, and `check --json --stdin`, `fmt --stdin`, `explain` on 1,500 fuzzed programs | **4,953 outputs identical**, 3 differing only in `\` in a printed path (a missing module's message), **0 different** |
| **The VS Code extension's tests** (`editors/vscode/test`, node's test runner), with the Linux `nexac` and with `nexac.exe` under Wine (URIs as VS Code writes them on Windows, `file:///z%3A/...`): finding `nexac` (setting, PATH, skipped relative folders, `.exe` only), the exact command (`nexac lsp`, no shell), explanations checked and turned into Markdown, quick fix topics; `activate()` with stand-ins for VS Code's API and the language client (registrations, how the server is started, the warning without `nexac`, a relative setting refused, the Explain command with the real `nexac`, a restart when the setting changes, `package.json`'s contributions); and two whole sessions through `vscode-languageserver-protocol` (the library VS Code's client uses) with `nexac lsp` and `nexac lsp --stdio`: a `\r\n` file with an emoji, a module beside it, a mistake (UTF-16 range, topic), hover, a fix, formatting that keeps `\r\n`, a module spelled with other capitals, save, close, an untitled file, shutdown | **12/12 pass** with each `nexac`; with `nexac.exe` also as part of the whole `tests/tooling.rs` run under Wine (**13/13**) |
| An earlier full run, made while Wine and the fuzzer ran beside it, had one Valgrind window scenario (Code Studio's `stop.steps`) time out waiting for the file chooser; the final run, alone on the machine, passed everything | noted: slow under load, not a failure of the code |
| `nexac build` / `run` on Windows (Wine) | the program is checked, then the message; exit 1 |
| Unwinding (the language server catches a compiler panic) in a Windows build made the same way | a caught panic under Wine: works |
| **`nexac.exe`**: built twice in different folders | byte-identical (sha256 `1b23521c…`), 1.6 MB, needs only Windows' own DLLs; ASLR (high entropy), DEP; no paths of the build machine in it |
| **The `.vsix`** (`vsce package`) | 413 KB: the extension, the grammar, `vscode-languageclient` and its 8 dependencies; no tests |
| **Fuzzing the compiler** (`tests/fuzz/fuzz.py`, every accepted mutant built with the **exact hardened `-O2 -Werror` flags** and run under ASan + UBSan + the leak counter, and through every 0.6 tool check) | **4,000 mutants** (seeds 2042-2045), **842 accepted** and run (161 window programs): **0 compiler crashes, 0 C build failures, 0 memory errors, 0 leaks**, 0 tool findings |
| **Fuzzing the tools** (`--tools-only`) | **10,000 mutants** (seeds 127-131): **1 finding** (seed 129: the first mistake shown changed when `fmt` removed an empty first line, because "no `fn main()`" was placed at line 1), fixed and re-run: 0 findings |

Found and fixed while building 0.9:
- On the first try on a real Windows PC (after the PR), `nexalang.path`
  was set to the folder holding `nexac.exe`, and the extension refused
  it. Extension 0.9.1 accepts the folder too (and quotes around the path,
  as Windows' "Copy as path" adds them); tested.
- `vscode-languageclient` adds `--stdio` to the server's arguments when
  the transport is stdio, and 0.6's `nexac lsp` refused any argument: the
  server would never have started in VS Code. Found by reading the
  client's code; `nexac lsp --stdio` is accepted now, and the extension
  gives no transport.
- A `file://server/share/x.nx` URI lost its server: on Windows the
  language server would have read `\share\x.nx` on the current drive.
- `nexac fmt` turned `\r\n` into `\n`, so on Windows every file was
  "not formatted".
- "the program has no `fn main()`" came first or second depending on an
  empty first line (fuzzing).
- Wine itself crashed on this machine (`free(): invalid pointer`) until
  `/run/user/0` existed: Wine's fallback for a missing runtime folder (not
  NexaLang's).

**0.8 (2026-11-18)** — Ubuntu 24.04 here: gcc 13.3, clang, GTK 3.24.41,
GtkSourceView 4.8.4, Xvfb, xdotool, Inter, JetBrains Mono, Valgrind.
`cd nexalang && cargo test`: **63 tests, all pass** (27 unit, 24 end
to end: one new, 12 in `tests/tooling.rs`). No earlier test was removed or
weakened; one expectation changed with the language: `ui_event_missing.nx`
now also lists `Edited`, `Output` and `Finished` among the events it
doesn't handle. The window tests now also run `tests/ui/programs.steps`,
`tests/ui/code_editor.steps` and Code Studio's 7 scenarios, three ways each.
`cd tools && cargo test -p nexa-run`: 7 pass (1 new: the theme's editor
colours are readable, read-only).

| Check | Result |
|---|---|
| Every earlier check (examples with gcc and clang, ASan + UBSan, Valgrind, window scenarios, theme, misuse, broker, modules, rejected programs, runtime errors, Focus Timer's 8 and Window Designer's 11 scenarios, every program Window Designer exported, the 0.6 tools, reproducible builds) | all still pass |
| **Colours**: `editor` is a standard module in both grammars; the sample has a line using it | GtkSourceView 4 itself and VS Code's `vscode-textmate` both give exactly `sample.expected` (the VS Code check now really ran here: `npm ci` in `tests/highlight`) |
| **`tests/ui/programs.steps`**: `start_program` and `stop_program` with real programs: output in order and stdout with stderr, input given and input not read, exit codes, a zero byte, no newline at the end, a 200,000-byte line of two-byte letters (in pieces, **0 letters cut**), 9 MB of output (8 MB, then the note), 1 MB of input; refused: not installed, no arguments, an empty name, a zero byte in an argument, a folder that isn't there; a folder; stopped (143), one that ignores SIGTERM (killed after 2 s: 137), stopped twice and after finishing (nothing happens), something left running that keeps the output open (finished after 1 s), 8 at once and a ninth refused, two at once with separate output; `ui.append` | exact output, 3 ways (leak counter; ASan + UBSan; Valgrind): 0 memory errors, 0 leaks from the program's own code |
| **`tests/ui/code_editor.steps`** (`use editor`): typing (Return keeps the indent, Tab is 4 spaces), `Edited` events, `replace_text` as one undo step (Ctrl+Z and Ctrl+Shift+Z), the same text again (nothing to undo), 5 marks (2 out of range ignored, past the end of a line, before its start), `go_to` past the end, new text that can't be undone, a zero byte, an emoji and a tab, Ctrl+Tab out of the editor, and the theme's `nexa.xml` written while it runs (the editor switches from the Horizon fallback to it) | exact output, 3 ways; 0 memory errors, 0 leaks from the program's own code |
| Misuse (5 new, 25 in all): `editor.mark` on a text, `ui.append` on an editor, a removed editor used, stopping a program that was never started, `start_program` before `ui.start` | each stops with its message in the program's own file and line |
| Rejected programs (5 new, 69 in all): `start_program`'s `!int` used as `int`, its arguments given as one text, `editor.mark` given text, `Edited` with two values, `editor` without `use editor` | refused with the expected message |
| **Code Studio, 7 scenarios driven by real key presses** (`apps/code-studio/tests`): normal use (a new program, a mistake typed, picked, explained, gone to, fixed, laid out, saved through the chooser, run: "The answer is 42"); Run and Stop (a program that never ends, stopped; F5 ignored while it runs; a program with a mistake: Lay out refuses, Run shows nexac's message); hostile files (2 MB, Latin-1, a zero byte, a 20,000-byte line: each refused with its message; an empty file, Windows line ends and a name with spaces and quotes open and run); unsaved changes (asked before closing, opening and starting again: Cancel keeps them, a name without `.nx` refused and nothing written, closing without saving keeps the file as it was); 360 px wide; a module with a mistake (listed with its file name; fixed on the disk, then run); without `nexac` on the PATH (said once, the list hidden, writing and saving still work) | 7 × 3 ways = **21/21**, exact output (the window's widgets, the text of the editor and the output view, and the files saved); 0 memory errors, 0 leaks from the program's own code |
| `nexac run`: output line by line through a pipe; nothing left in the temporary folder while it runs, after it's stopped (SIGTERM to its process group) or when the build fails | new test, passes |
| **Its package**: `nexa-pack build` twice in different folders | byte-identical (sha256 `e1682bc0…`), `nexa-pack verify`: `"ok":true`, `"depends":["gtk+3.0","gtksourceview4"]`, `"permissions":["files.documents"]`, unsigned; PIE, `BIND_NOW`, RELRO, fortify (`__*_chk`); linked with `libgtksourceview-4` (Focus Timer and Window Designer aren't) |
| **The extracted program** (`nexa-pack extract`), normal scenario | exactly the expected output |
| **Installed, confined by the real `nexa-run`** (Landlock; only `files.documents`; home folder outside `/tmp`; `nexac` in `/usr/local/bin` as on NexaOS) | `nexa-run --explain`: its own folders, `files.documents`, no internet. Opening from Documents, mistakes as you type, Explain, Lay out and saving work; its colour files are written to `~/.cache/code-studio/nexalang/`; saving in the home folder is refused with the window's message; **Run builds the program and then can't start it**: the output view shows nexac's message (as designed: see the decisions) |
| **Fuzzing the compiler** (`tests/fuzz/fuzz.py`, seeded also with the new tests and Code Studio, with `editor.*`, `ui.append`, `start_program`, `stop_program` and the new events among the inserted pieces; programs that use `editor` built with GtkSourceView's flags): every accepted mutant built with the **exact hardened `-O2 -Werror` flags** and run under ASan + UBSan + the leak counter, and through every 0.6 tool check | **9,000 mutants** (seeds 2039-2041), **1,970 accepted** and run (398 of them window programs): **0 compiler crashes, 0 C build failures, 0 memory errors, 0 leaks**, 0 tool findings |
| **Fuzzing the tools** (`--tools-only`) | **30,000 mutants** (seeds 117-126): 6,378 accepted, 8,582 formattable (5,932 re-laid out), 25,733 mistakes explained: **0 crashes, 0 non-idempotent formats, 0 changes of meaning, 0 disagreements** |

**Screenshots** (`shots/0.8/`, all looked at): Code Studio empty, a new
program, a mistake underlined and listed, the explanation, after running,
running, stopped, a program with a mistake that can't be laid out and its
run, a file too big, the question before closing without saving, 360 px
wide (empty and in use), and installed (Run refused; saving outside
Documents refused). No clipped text, no default grey GTK, no dark-on-dark.

Found and fixed while building 0.8:
- **under Valgrind, a program that failed to start printed the window
  program's earlier output a second time** (the child got a copy of the
  unwritten stdout buffer). Output is now flushed before every start, and
  a folder that isn't there is refused before starting anything;
- `nexac run` left an empty `/tmp/nexac-PID` folder when the build failed
  (and its `.nexa.c` when GTK was missing), and the built program when it
  was stopped; now nothing is left in any of those cases (tested);
- a running program's output came through a pipe 4 KB at a time, so
  Code Studio showed nothing until it ended (`NEXALANG_LINE_OUTPUT`);
- the rounded corners of code views (0.7) were covered by the text inside;
  explanations were cut off at the right edge of a code view; a window
  with an editor, a list and a code view grew past 640 px;
- after a check, "No mistakes." replaced what Run had just said (seen
  under Valgrind, where the check finished later): the status line now
  only changes when the number of mistakes does;
- "can't be laid out until this mistake is fixed: error: ..." said
  "error" twice;
- GTK refused the first `font-feature-settings` (it wants HarfBuzz's
  syntax, `"-liga, -calt"`);
- Valgrind reports conditional jumps in the code PCRE2 compiles at run
  time for GtkSourceView's patterns (no object to read its definedness
  from); suppressed only for frames with no object at all
  (`tests/ui/valgrind.supp`);
- the test driver: file names with spaces (`\x20`), `# env:` and
  `# tree: full`.

**0.7 (2026-11-18)** — Ubuntu 24.04 here: gcc 13.3, clang, GTK 3.24.41,
Xvfb, xdotool, Inter, JetBrains Mono, Valgrind, Sway 1.9 + grim.
`cd nexalang && cargo test`: **62 tests, all pass** (27 unit, 23 end to
end: one new, 12 in `tests/tooling.rs`). No earlier test was removed or
weakened; the window tests now also run `tests/ui/remove.steps` and
Window Designer's 11 scenarios, three ways each.

| Check | Result |
|---|---|
| Every earlier check (examples with gcc and clang, ASan + UBSan, Valgrind, window scenarios, theme, misuse, broker, modules, rejected programs, runtime errors, Focus Timer's 8 scenarios, the 0.6 tools, reproducible builds) | all still pass |
| **`tests/ui/remove.steps`** (`remove`, `clear`, 500 rebuilds of a row, a removed widget's shortcut gone and the main button made again, `code` with braces and a zero byte, `to_clipboard` then Ctrl+V into a text field, `save_file` (starts in Documents, saves, asks before replacing: cancelled), `open_file` with `*.txt`, the window's widgets printed before and after) | exact output, 3 ways (leak counter; ASan + UBSan; Valgrind): 0 memory errors, 0 leaks from the program's own code |
| Misuse (6 new, 20 in all): a removed widget used (after `remove` and after `clear`), removing the main column, clearing a text, checking a code view, a second main button after clearing another row | each stops with its message in the program's own file and line |
| Rejected programs (3 new, 64 in all): `save_file`'s `?str` used as `str`, `code` given a number, `clear` used as a value | refused with the expected message |
| **Window Designer, 11 scenarios driven by real key presses** (`apps/window-designer/tests`): normal use (title, app id, a heading, a text field, a row with two buttons, the main button moved first, a list moved out of the row, a check box turned on, a second main button refused, the preview, the code, Export through the file chooser, Copy); editing at the edges (moving past the ends, Into and Out where they can't, removing a row with a part in it: asked, cancelled, then accepted; no title; an app id that can't be used; a name without `.nx`; a cancelled export; 360 px wide); a saved design with every kind of part, three levels of rows and columns and words to escape (`"`, `{}`, `\`, an emoji, a tab, 260 bytes); an empty file; a file cut off; not UTF-8; 3 MB; a part inside a button; two main buttons; an unknown kind; 201 parts | 11 × 3 ways = **33/33**, exact output (every window's widgets and status line, and the files it saved); 0 memory errors, 0 leaks from the program's own code; files it couldn't read are left byte for byte as they were |
| **Every program Window Designer exported in those tests** (3 designs: the notes window, the every-part window, the empty window) | accepted by `nexac check` and `nexac fmt --check`, built with the hardened flags, and when run show **exactly** the widgets the designer's preview showed (compared as text) |
| **Its package**: `nexa-pack build` twice in different folders | byte-identical (sha256 `f9983fd7…`), `nexa-pack verify`: `"ok":true`, `"permissions":["files.documents"]`, unsigned; PIE, `BIND_NOW`, fortify |
| **Installed, confined by the real `nexa-run`** (Landlock; the extracted program; only `files.documents`; home folder outside `/tmp`) | `nexa-run --explain` lists its own folders, `files.documents`, no internet. The normal scenario gives exactly the same output, and the exported program is byte for byte the expected one, in `~/Documents`. Exporting to the home folder itself is refused by Landlock, and the window says where it can save. GTK can't write `recently-used.xbel` there and prints two warnings (see the known limits) |
| **Wayland**: the extracted app in a headless Sway (pixman), screenshot with grim | draws correctly; looked at |
| **Fuzzing the compiler** (`tests/fuzz/fuzz.py`, seeded with the new window programs and Window Designer, with `ui.clear`, `ui.remove`, `ui.code`, `ui.to_clipboard`, the file choosers among the inserted pieces): every accepted mutant built with the **exact hardened `-O2 -Werror` flags** and run under ASan + UBSan + the leak counter, and through every 0.6 tool check | **9,000 mutants** (seeds 2036-2038), **1,942 accepted** and run (286 of them window programs): **0 compiler crashes, 0 C build failures, 0 memory errors, 0 leaks**, 0 tool findings |
| **Fuzzing the tools** (`--tools-only`: `check --json`, `fmt` twice and the C of the formatted program, `explain`, a language server session) | **30,000 mutants** (seeds 107-116): 6,418 accepted, 8,636 formattable (5,930 re-laid out), 25,795 mistakes explained: **0 crashes, 0 non-idempotent formats, 0 changes of meaning, 0 disagreements** |

**Screenshots** (`shots/0.7/`, all looked at): Window Designer empty,
editing, after exporting, the preview, the code, the export chooser, the
question before removing a row, a damaged file and the question before
replacing it, an unreadable file, a file that isn't a window, the
every-part design (editing, preview, code), the empty preview, 360 px wide
(editing and preview), installed and saving outside Documents, on Wayland;
and `ui`'s save and open choosers and GTK's "replace it?" question. No
clipped text, no default grey GTK, no dark-on-dark.

Found and fixed while building 0.7:
- GTK's "replace it?" question (asked by the save chooser) was light grey
  with light text: unreadable. `ui` now gives GTK's own dialogs Hexa's
  colours;
- the file chooser came out bigger than the screen with its title bar
  off the top (GTK sizes it from settings that may be missing), and its
  own title bar never showed: it's now sized to the screen and placed
  over its window;
- JetBrains Mono showed `->` and `==` as joined symbols in the code view;
  a beginner would copy the wrong thing. Ligatures are off there;
- a list inside a row inside a column stayed 120 px tall in any window;
- `ui.focus(list)` left the keyboard on the list itself, where arrow keys
  did nothing;
- switching GTK's recent files off (tried first, to keep the chooser out
  of `~/.local/share`) made GTK write an **empty** `recently-used.xbel`,
  wiping every app's recent files; that was reverted (see the decisions);
- Valgrind reported glibc's AVX2 `wcpncpy` reading past a file name when
  the chooser sorts names: a known false alarm of that optimised copy;
  suppressed for exactly that path (`tests/ui/valgrind.supp`);
- the test driver: under Valgrind, a key could reach the window while a
  file chooser was still closing; `gone TITLE` now waits for a dialog to
  close;
- Window Designer itself, from its tests and screenshots: the preview of
  an empty design showed the designer's own note as if it were part of
  the window; "The 1 parts inside it go too"; old messages stayed in the
  status line after the next change worked; the buttons at the bottom
  stopped half-way down in a short preview (they're at the top now).

**0.6 (2026-11-18)** — Ubuntu 24.04 here: gcc 13.3, GTK 3.24.41,
GtkSourceView 4.8.4, Xvfb, Node 22 with `vscode-textmate` 9.2.0 (VS Code's
grammar engine), Valgrind. `cd nexalang && cargo test`: **61 tests, all
pass** (27 unit, 22 end to end as before, 12 new in `tests/tooling.rs`).
No earlier test was removed or weakened.

| Check | Result |
|---|---|
| Every earlier check (examples with gcc and clang, ASan + UBSan, Valgrind, window scenarios driven by keys, theme, misuse, broker, modules, rejected programs, runtime errors, Focus Timer's 8 scenarios × 3 ways, reproducible builds) | all still pass |
| **`nexac fmt` keeps the meaning**: every `.nx` file in the repository (examples, window examples, every test folder, the module tests, `apps/*/src`), its layout first messed up (no indentation, extra spaces and tabs, spaces at line ends, more empty lines), then formatted in place | **126 files**: each comes back **exactly** as it was; **50 programs give byte-identical generated C** (`emit-c --no-lines`), **72 rejected programs give the same mistake**; formatting again changes nothing; every comment kept |
| The standard modules, Focus Timer, the examples and `nexa-pack new`'s template are formatted | yes (`fmt --check`); formatting them changed only the spacing of lined-up comments in `std/ui.nx` and Focus Timer, and two empty lines at the end of `std/json.nx` |
| `nexac fmt` command line: `--check`, in place (permissions kept, nothing left behind, a link's target replaced and the link kept), `--stdin`; files that don't parse, aren't UTF-8, aren't `.nx`, don't exist | as designed; a file that doesn't parse is never changed |
| **`nexac check --json`** against the text output, for every rejected test program and module test (73 files) | same message, file, line, column and hint, in order; every mistake has a topic; a file that can't be read has line 0; `--stdin` (unsaved text, modules from the disk) reports the right line and column |
| **Every message in the compiler has a topic**: a unit test reads all ~130 messages from the source; the fuzzer checks them as shown | 0 without a topic (the fuzzer found two shapes the source scan missed, "can't change this" and "can't \`push\` to this", now covered) |
| **`nexac explain`**: each topic's refused program is refused **with that topic**, its fixed program is accepted and formatted; a message instead of a topic; a typo | 19/19 examples; typo → exit 2 with the list |
| **`nexac lsp`**: a whole session (initialize, open with a mistake after an emoji: UTF-16 column checked, hover explains, change, formatting edit, close, shutdown, exit 0); a module with a mistake; requests before initialize; bad JSON, bad UTF-8, no Content-Length, a bad Content-Length, wrong types, an unknown method, 100,000-deep JSON, a 64 MiB + 1 byte message (skipped, the next one answered), a file that isn't open; input ending without shutdown (exit 1) | all answered as the protocol says; never a crash |
| Valgrind on `nexac lsp` (a session over Focus Timer: open, a mistake, hover, formatting, shutdown), `fmt`, `check --json`, `explain` | 0 errors, 0 leaks |
| **Colours**: `tests/highlight/sample.nx` (every kind of word, escapes, a bad escape, values in text with strings and struct literals inside, numbers in every form, `0..10`) coloured by **GtkSourceView 4 itself** (a C program, every style its own colour) and by **`vscode-textmate`** with the TextMate grammar | both give **exactly** `sample.expected`, character by character; `app.nx` is recognised as NexaLang |
| **NexaOS's own text editor** (`text-editor/`, with the real `nexa-textio` engine) opening a `.nx` file under Xvfb | status bar says "NexaLang", colours readable on the Hexa theme; screenshot looked at (`shots/0.6/text-editor-colours.png`) |
| **Fuzzing the tools** (`tests/fuzz/fuzz.py --tools-only`, mutations now also change layout: indentation, tabs, comments with emoji, empty lines, lines split after operators): every mutant through `check --json` (read back by Python's `json`), `check`, `explain` of every message, `fmt` twice and the C of the formatted program, and a language server session (diagnostics, formatting, hover, shutdown) | **18,000 mutants** (seeds 101-106) on the final compiler: 3,966 accepted, 5,217 formattable (3,530 of them re-laid out), 15,153 mistakes explained: **0 crashes, 0 non-idempotent formats, 0 changes of meaning, 0 disagreements** between `check`, `check --json` and the language server |
| **Fuzzing the compiler** as before (the lexer and the message plumbing changed): every accepted mutant built with the **exact hardened `-O2 -Werror` flags** and run under ASan + UBSan + the leak counter, plus all the tool checks above | **9,000 mutants** (seeds 2033-2035), **1,922 accepted** and run (215 of them window programs): **0 compiler crashes, 0 C build failures, 0 memory errors, 0 leaks**, 0 tool findings. (Run before the language server's header fix below, which only touches broken message headers; the 18,000 above ran on the final compiler) |

Found and fixed while building 0.6:
- a message shape without a topic (above);
- the language server's header reader: after a bad `Content-Length` it
  stopped before the end of the headers, so the next message was lost
  (found by its unit test);
- formatting first put one space before trailing comments, which undid
  the lined-up notes in `std/ui.nx` and Focus Timer: comments after code
  now line up in groups;
- one explanation's example (strings inside strings) was one level short
  of the limit it was meant to show, so it was accepted (found by the
  example test).

**0.5 (2026-11-17)** — Ubuntu 24.04 here: GTK 3.24.41, Xvfb, xdotool,
Sway 1.9 + grim (headless Wayland), Inter, JetBrains Mono, gcc 13.3,
clang, Valgrind. `cd nexalang && cargo test`: **35 tests, all pass**
(13 unit, 22 end to end; 2 new). `cd tools && cargo test -p nexa-pack`:
**19 pass** (3 new: the template's files, escaping, refused ids).

| Check | Result |
|---|---|
| Every earlier check (examples with gcc and clang, ASan + UBSan, Valgrind, window scenarios, theme, misuse, broker, modules, rejected programs, runtime errors) | all still pass; no test removed or weakened |
| New example `sizes.nx` (`files.size`: empty, small, big, missing, a folder, a zero byte, an empty path; refusing a file that's too big) | exact output, gcc and clang, every byte freed, clean under ASan + UBSan and Valgrind |
| Bad programs rejected with the expected message | 61/61 (2 new: `files.size` used without handling its error; `ui.display` given a number) |
| `nexac build -o build/deeper/app` makes the folders; a folder that can't be made is a message, exit 1 | pass |
| **Programs built twice in different folders are byte-identical** (`notes.nx`, `apps/focus-timer`) | identical |
| **`nexa-pack new my-app --lang nexa --name 'My "App" {x}'`** → `nexa-pack build` → verify → extract → run the extracted program and drive it with key presses | builds with the hardened flags, `"ok":true`, the window shows the name exactly, Enter greets; screenshot looked at |
| **`apps/focus-timer`, 8 scenarios driven by real key presses** (`apps/focus-timer/tests/*.steps`: two sessions with a pause, keyboard shortcuts, a length it can't read, 360 px wide, an empty file, a file cut off half-way, a file that isn't UTF-8 (cancel, then accept "Clear history"), a 3 MB file, nonsense entries and numbers too big for an `int`, closing while the timer runs). The saved file is compared exactly | 8 × 3 ways (hardened build + leak counter; ASan + UBSan; Valgrind) = **24/24**, 0 memory errors, 0 leaks from the program's own code, no GTK warnings |
| **Its package**: `nexa-pack build` twice | byte-identical (sha256 `9e1ba85e…`), `nexa-pack verify`: `"ok":true`, `"permissions":[]`, unsigned |
| The program in the package: PIE, full RELRO (`BIND_NOW`), stack protector, fortify (`__*_chk`) | all present (readelf) |
| **Run from the extracted package, confined by the real `nexa-run`** (Landlock, built from `tools/nexa-run`, the app registered with no permissions, home folder outside `/tmp`) | all 8 scenarios give exactly the same output; `nexa-run --explain` lists only its own folders and "no internet". A probe program under the same confinement could **not** read a file in the home folder, and could use its own data folder |
| **Wayland**: the extracted app in a headless Sway (wlroots, pixman), screenshot with grim | draws correctly with JetBrains Mono and Inter; looked at |
| A home folder with spaces, quotes and accents in its name | works |
| Start-up: window shown after | ≈0.5 s on this machine (the measurement's own polling step is 0.5 s) |
| **Fuzzing** (`tests/fuzz/fuzz.py`, now also seeded with `apps/*/src/main.nx` and `files.size` / `ui.display`): every accepted mutant built with the **exact hardened `-O2 -Werror` flags** and run under ASan + UBSan + the leak counter | **12,000 mutants** (seeds 2029-2032), **1,004 accepted** and run (78 of them window programs): **0 compiler crashes, 0 C build failures, 0 memory errors, 0 leaks** |

**Screenshots** (`shots/0.5/`, all looked at): Focus Timer's empty state,
running, paused, history, a length it can't read, an unreadable file, a
damaged file, a file too big, the question before replacing it, after
clearing, nonsense entries, closing while running, 360 px wide, on
Wayland; and the `nexa-pack new` template. No clipped text (long names
end in "…"), no default-grey GTK, no dark-on-dark.

Found and fixed while building 0.5:
- `nexac build -o build/app` failed when `build/` didn't exist yet (every
  new project's first build);
- the test driver couldn't compare saved files that aren't UTF-8 (it now
  shows those bytes as `\xNN`);
- Focus Timer itself, from looking at the screenshots: totals under a
  minute showed as "0 min"; errors repeated the long path of the history
  file; an empty or cut-off file showed a JSON error with line and column
  (now: an empty file is an empty history; a cut-off one is "damaged");
- `nexa-pack new` put the app's name into `nexa.toml` and the program
  without escaping it, and accepted ids the build would refuse later.

**0.4 (2026-11-17)** — GTK 3.24.41 really built and run here (Ubuntu 24.04
packages: libgtk-3-dev, Xvfb, xdotool, Inter), gcc 13.3, clang, Valgrind.
`cd nexalang && cargo test`: **33 tests, all pass** (13 unit, 20 end to
end; the window tests skip with a message where GTK, Xvfb or xdotool are
missing).

| Check | Result |
|---|---|
| 14 example programs + the three-file program: exact output with gcc and clang under `-Werror`, every byte freed; AddressSanitizer + UBSan + LeakSanitizer; Valgrind | 15/15, 0 errors |
| Window programs (`examples/ui`, `tests/ui`) built by `nexac build` with every hardened flag + GTK's, gcc and clang; a program without `ui` isn't linked with GTK | 4×2 build clean; `hello` has no GTK |
| **Windows driven by real key presses** on a headless X server (`tests/ui/*.steps`: typing, Enter, Tab, arrows, Space, shortcuts, Esc, Ctrl+Q, dialogs): exact expected output, run 3 ways: hardened build + leak counter; ASan + UBSan; Valgrind | 7 scenarios × 3 = **21/21**, 0 memory errors, 0 leaks from the program's own code, no GTK warnings |
| The window leak check itself: a leak planted in the runtime (a tooltip's text never freed) | caught by both ASan and Valgrind |
| Churn: 1,500 rounds of rebuilt lists, text set and read back, events | every NexaLang byte freed; clean under ASan and Valgrind |
| Misusing `ui` (14 kinds: wrong widget, wrong kind, second primary, bad/reserved/duplicate shortcut, before `start`, twice `start`, row out of range, `wait_ms(0)`, ...): stops with a message **in the program's own file and line** | 14/14 |
| No display: `error: can't open a window: there's no display to show it on`, exit 1, nothing leaked | pass |
| **Theme**: a theme set before start, then `~/.config/gtk-3.0/gtk.css` replaced while the window is open: background and accent checked pixel by pixel | follows both, live |
| Broker protocol against the stand-in broker (the real reply format of nexa-brokerd): result, refusal, unreadable and non-object replies, closed connection, no result, locked account, unverified package (`call` and `ask`), password needed, right and wrong passwords, lockout after three, no broker | 19/19 |
| The password dialog in a window (`ui.broker_call`): wrong password then right, cancel, three wrong tries, lockout | exact output; screenshots looked at |
| Bad programs rejected with the expected message | 59/59 (6 new) |
| Runtime errors stop with exit 1 and file:line | 12/12 (running out of stack is new) |
| Module mistakes in the right file | 11/11 |
| Generated C reproducible | all examples and window programs |
| **Fuzzing** (`tests/fuzz/fuzz.py`, now in the repository): mutated programs (the examples, the window programs, the tests); every accepted one built with the **exact hardened `-O2 -Werror` flags** and run under ASan + UBSan + the leak counter (window programs on a headless X server) | first 10,000: **3 findings**, all fixed (below). On the final compiler: **14,000 mutants** (seeds 2026, 2027, 2028), **1,121 accepted** and run (84 of them window programs): **0 compiler crashes, 0 C build failures, 0 memory errors, 0 leaks** |

**Screenshots** (`shots/0.4/`, all looked at): empty state, normal use,
keyboard focus, a confirm dialog, unreadable saved data (error), the
password dialog (asking, wrong password, locked out), every widget, the
same window after a live theme change, and 360 px wide. No clipped text,
no default-grey GTK, no dark-on-dark.

Found and fixed while building 0.4:
- **the fuzzer**: a function that always calls itself made gcc's
  `-Winfinite-recursion` fail the build under `-Werror` ("bug in nexac";
  there since 0.1). nexac now refuses such functions with a message, the
  generated C no longer lets the C compiler's own (different) analysis
  fail the build, and recursion that runs too deep anyway stops with
  "the program ran out of stack space" (before, the program died with no
  message at all, and `nexac run` hid the signal);
- **the fuzzer**: `print(1e308)` made gcc's `-Wformat-truncation` fail
  the build (there since 0.2): the float-printing buffer now fits any
  number gcc can imagine;
- runtime errors inside a standard module were reported at a line of the
  module (`std/ui.nx`), not the program's;
- a type with the same name as a standard module's type was reported in
  the module's file, where it can't be fixed;
- windows: Tab walked through every row of a list (now it leaves the
  list; arrow keys move inside); rebuilding a list lost the keyboard
  focus; the primary button took Enter away from list rows; a dialog lost
  its window title;
- the test driver: under load, polling the X server every 100 ms filled
  its connection queue, and a program's own connection failed (EAGAIN)
  now and then; it now waits on one connection.

**0.3 (2026-11-17)**

| Check | Result |
|---|---|
| Compiler unit tests | 13 pass |
| 13 example programs + a three-file program: exact output with gcc and clang under `-Werror`, every byte freed | 14/14 |
| Same programs under AddressSanitizer + UBSan + LeakSanitizer, and Valgrind | 0 errors |
| Bad programs rejected with the expected message | 53/53 |
| Module mistakes (private function/type, missing `use`, circle, missing file, clash with a standard module, type defined in two files, error inside a module, pub item exposing a private type, `__` primitive, `use` after code) reported in the right file | 11/11 |
| `fn main() -> !void` ending in an error: message, exit 1, nothing leaked | pass |
| Broker client against a stand-in broker: reply, unknown action, garbage reply, no reply, empty action, broker not running | pass |
| **JSON differential test vs Python's `json`**: 48,000 generated documents, valid and broken | 0 disagreements, 0 crashes (two deliberate differences: big whole numbers round to `float`; numbers too big for a `float` are refused) |
| JSON hostile input: 20,000 random strings, 100,000-deep nesting | 0 crashes; refused cleanly past 200 levels |
| **Fuzzing**: 4,000 mutated programs; every accepted one built with the **exact hardened `-O2 -Werror` flags** and run under sanitizers + leak counter | 0 compiler crashes, 0 build failures, 0 memory errors, 0 leaks |

Found and fixed while building 0.3:
- a call nested in another call's arguments inside a module lost its
  module (`join_path(home(), ...)`); found by the standard library itself;
- `json.parse` could stop the program on `tru` followed by a non-ASCII
  letter (found by the differential fuzz); now compared byte by byte;
- nexac itself crashed on program arguments that weren't UTF-8;
- gcc's `-O2` overflow analysis rejected one runtime function under
  `-Werror` (a false alarm, now ruled out in the code); the fuzzer now
  builds with the real flags, which is how it would have been caught;
- two over-strict rules were dropped after they blocked natural code:
  values may now share a name with a function (`let ok = ...`, a
  parameter called `value`);
- clearer hints for `{name}` written outside the quotes and for `print`
  with two values.

**0.2 (2026-11-16)**

| Check | Result |
|---|---|
| Compiler unit tests | 13 pass |
| 10 example programs: exact output with **gcc and clang** under `-Werror`, **and every byte freed** (`--check-memory`) | 10/10 |
| Same examples under **AddressSanitizer + UBSan + LeakSanitizer** | 0 errors |
| Same examples under **Valgrind** (`--leak-check=full`, errors on any leak) | 0 errors |
| `ownership.nx`: the known hard cases for reference counting (`x = x`, `xs[0] = xs[1]`, `push(xs, pop(xs))`, a `match` replacing its own subject, replacing a list while looping over it, returning parameters' fields, leaving nested loops from inside a `match`) | correct output, checked by hand; clean under all three tools |
| The leak counter itself: one release removed from generated code | detected (exit 3) |
| Stress: 2,000,000 rounds of making/dropping text, lists, structs, enums, `?T` | **peak 2.3 MB**; the same program with freeing switched off runs out of a 64 MB cap |
| Bad programs rejected with the expected message and position | 39/39 |
| Runtime errors stop with exit 1 and file:line | 11/11 |
| Generated C reproducible (byte-identical) | all examples |
| **Fuzzing**: 5,600 mutated programs; every accepted one built with sanitizers + leak counter and **run** | 0 compiler crashes, 0 C build failures, 0 memory errors, 0 leaks (345 ran) |

**0.1:** 12,000 fuzzed programs, 0 crashes; hostile nesting refused cleanly.

Found and fixed while building 0.2: strings inside `{...}` were refused,
which made the new text functions awkward (now allowed, 8 levels max);
floats printed as `1e+03` (now `1000.0`).

## 6. Known limits of 0.9

Windows and VS Code (0.9):
- **Not tried on a real Windows PC or in a real VS Code.** This machine
  couldn't download VS Code or VSCodium (their servers are blocked here),
  and has no Windows. `nexac.exe` ran under Wine 9.0 (every 0.6 tool test,
  the comparison with the Linux `nexac`, the extension's sessions with
  it); the extension ran in Node.js against a stand-in for VS Code's API
  and the real `nexac`. What only a real VS Code can show (the extension
  loading, the colours on screen, the light bulb, format on save happening
  on Ctrl+S) is untested. Please try it once: the steps are in
  `editors/README.md`.
- **On Windows, programs can't be built or run** (`nexac build`/`run` say
  so: see the decisions). Checking, layout, explanations and the language
  server work. WSL (a Linux inside Windows) can probably run the Linux
  `nexac` fully; not tried.
- `nexac.exe` needs 64-bit Windows 10 or later (what Rust supports); no
  32-bit or ARM build.
- The `.vsix` isn't published to the VS Code Marketplace or Open VSX
  (that needs an account and a publisher token: the author's decision);
  it's installed from the file. It isn't byte-for-byte reproducible (the
  zip keeps file times); `nexac.exe` is.
- The extension has no Run or Build command (they'd only work on Linux),
  no completion or go-to-definition (the language server doesn't have
  them yet: 0.6's limits).
- A path in a message about a missing module is written Windows' way
  (`z:\work\helpers.nx`) on Windows; every other message is the same.
- Under Wine, file names with letters outside ASCII need a UTF-8 locale on
  the Linux side (`LANG=C.UTF-8`); on Windows itself names are Unicode.

## Known limits of 0.8 (still true)

Code Studio, `editor` and programs (0.8):
- **Installed, Code Studio can't Run**: NexaOS's app confinement
  (`nexa-run`, Landlock) never lets an app run a file it has written, so
  the program `nexac run` builds can't start; the output view shows
  `nexac`'s message ("an installed NexaOS app can't run programs it
  builds; run `nexac run FILE.nx` in a terminal"). Checking, laying out and
  explaining work installed. Allowing it would need a new, deliberate
  NexaOS permission (see the decisions).
- **Building needs a C compiler and GTK's headers**: `nexac run` needs
  `gcc`, and window programs `gtk+3.0-dev` (and `gtksourceview4-dev` for
  `use editor`); a NexaOS image doesn't have them unless they're added
  (`build-base`). Code Studio shows `nexac`'s message when they're missing.
- Code Studio edits one file at a time; a module it uses is checked from
  the disk (a mistake there is listed with its file name). No search and
  replace, no completion, no "go to definition"; the window's title
  doesn't show the file name (the line above the editor does).
- Programs started by `start_program` read only the input given (then end
  of input): Run can't type into a program that asks with `read_line`.
  Something a program leaves running after it ends (`sleep 5 &`) isn't
  stopped (only what's still running when Stop is pressed or the window
  closes).
- `Edited` coalescing (one waiting at a time) is there to bound the queue;
  with GTK handling one key per turn of the loop it hasn't been seen to
  merge two key presses in the tests.
- Picking a file whose name isn't UTF-8 is refused by the chooser (as in
  0.7); not driven by a test.

## Known limits of 0.7 (still true)

Window Designer and `ui` (0.7):
- **Window Designer makes the window, not what it does**: the program it
  writes has a place for each button, text field, check box and list, and
  a status line saying what was used. It can't set tooltips, shortcuts,
  `mono`, `display` or danger buttons, has no undo, and can't read a
  `.nx` file back in (the design file is what it keeps). Parts are moved
  with keys and buttons, not dragged.
- The preview fills the designer's window, so it's as wide as that window,
  not the 560 px a new program starts with.
- **The file chooser can only save where the app may**: installed, an app
  without `files.documents` (or another `files.*` folder) can only save in
  its own folders. While it's confined, GTK can't note the chosen file in
  `recently-used.xbel` and prints two warnings on stderr (see the
  decisions). The chooser keeps its own settings (size, sort order) with
  GTK's settings system (dconf on NexaOS); that's GTK's behaviour, not the
  app's.
- `ui.code` is read-only; the editable code widget is `editor` (0.8).
- Not tested here: picking a file whose name isn't UTF-8 (the chooser
  refuses it with a message and stays open; such a name can't be typed by
  the test driver), and the clipboard after the program has ended (GTK
  hands it to a clipboard manager if the desktop has one).
- `NEXALANG_UI_TREE` prints what a test needs (kinds, words, states, list
  rows), not sizes or colours; those are checked by looking at screenshots.

Tools (0.6):
- **`nexac fmt` doesn't wrap long lines** (moving a line break could
  change the program), and doesn't touch the code inside a string's
  `{...}`.
- **The language server is small**: mistakes, formatting and explanations
  only; no completion, go-to-definition or renaming yet. It wants the
  whole text on every change (it says so when it starts; an editor that
  sends only the changed part anyway is ignored until the next whole
  text). A mistake in a module the open file uses is shown on the first
  line, naming the module's file and line; modules are read from the disk,
  so unsaved changes in another open module aren't seen until it's saved.
  An unsaved, never-named buffer can't `use` modules of its own.
- A lone carriage return (`\r` without `\n`) is a line break to LSP
  editors but not to NexaLang; positions after one on the same line are
  off. Files from Windows (`\r\n`) are fine.
- **Nexa Mind** doesn't call `nexac explain` by itself (see the decisions:
  its tools don't run programs). The explanations are ready for it
  (`nexac explain --json`).
- The NexaOS text editor (Python, GtkSourceView 4) colours `.nx` files
  once `nexalang.lang` is installed (`scripts/stage-text-editor.sh`); it
  doesn't show mistakes as you type. Code Studio (0.8) does.
- The VS Code grammar is tested with VS Code's own engine, not inside VS
  Code (0.9's extension: see above).

The language and `ui` (the language unchanged since 0.5):

- **One window per program** (dialogs aside). Widgets can be removed
  since 0.7 (`ui.remove`, `ui.clear`).
- **List rows are text only**; no images, menus, tabs, sliders or drag and
  drop yet. The file chooser (0.7) picks one file at a time; no folders.
- **No hexagon shape** inside windows yet (the NexaOS mark for one or two
  things per screen); apps carry it in their icon (`nexa-pack new` makes one).
- Widgets are `int`s: passing a count where a widget is meant is caught
  only when the program runs (with a message), not by the compiler.
- **A password is ordinary NexaLang text** after the dialog: it goes only
  into the request to the broker and is freed afterwards, but freed memory
  isn't wiped (GTK wipes its own copy in the dialog).
- **Tested on Ubuntu (glibc) here, not on Alpine (musl).** This machine
  couldn't reach Alpine's servers or Docker Hub, so the packages built
  here hold a glibc program; NexaOS's packages come from
  `scripts/build-apps.sh` in the Alpine builder, which now also builds
  `nexac` (`scripts/build-native.sh`). Those two script changes are
  untested here (no Docker images could be downloaded).
- Key presses are tested on X11 (Xvfb); on Wayland (headless Sway) the
  app was started and looked at, but not driven by keys.
- Not done here: the live-boot test in QEMU
  (`scripts/test-live-boot.py`, needs a NexaOS ISO), and signing with the
  release key (only on the release machine). `apps/focus-timer`'s package
  is unsigned until `make nexa-apps` builds it there.
- **Single-threaded.** Reference counts aren't atomic; threads (later)
  will use message passing, never shared values. (A window's broker
  request waits on a helper thread that never touches NexaLang values.)
- No generic functions or types of your own (only the built-in `?T`,
  `!T` and `[T]`).
- `s += "x"` in a loop copies the text each time; collect pieces in a list
  and `join` them.
- No network access yet (only the broker's local socket).
- Without a window, `broker` reports `NeedsPassword` but can't ask for the
  password itself (reading one from a terminal without showing it isn't
  in the language yet).
- `files` doesn't decide what an app may touch: installed apps are
  confined by `nexa-run` (Landlock) to the folders their permissions name.
- Enums can't be compared with `==` when a variant holds values.
- Nesting deeper than 200 levels in one expression or block is refused;
  source files are limited to 999,999 lines.

## 7. Roadmap

| Version | What | Done when |
|---|---|---|
| **0.1** ✅ | Core language, checked ints, bounds-checked lists, structs, compile to hardened C | §5 |
| **0.2** ✅ | Reference counting (no leaks, flat memory), value-semantics lists, `enum` + exhaustive `match`, `?T` instead of null, text functions, `read_line`/`args` | §5 |
| **0.3** ✅ | `!T` errors and `?`; modules with `pub`; standard modules `files`, `time`, `json`, `broker`; `match` on `int`/`str`; every `str` valid UTF-8 | §5 |
| **0.4** ✅ | **`ui`: GTK 3 windows with Hexa built in** (theme colours with the fallback block, live theme following, Inter / JetBrains Mono, one primary action, empty states, keyboard use); the program-driven event loop; password prompts and the broker's real protocol (`BrokerReply`); functions that always call themselves refused; running out of stack reported | §5. (WRITING_NEXA_APPS.md §6 for a packaged app is 0.5's sample app) |
| **0.5** ✅ | `nexa-pack new my-app --lang nexa`; `.nexa` packages built from `.nx`, reproducibly; `apps/focus-timer`, a NexaOS app in NexaLang; `files.size`, `ui.display` | §5. In the Store once `make nexa-apps` builds and signs it on the release machine |
| **0.6** ✅ | `nexac fmt` (meaning-preserving, idempotent, keeps comments); `nexac check --json`; `nexac explain` (20 topics; 19 with a refused and a fixed program, both tested); `nexac lsp` (diagnostics, formatting, hover); colours for GtkSourceView 4 (NexaOS's text editor) and VS Code | §5. Nexa Mind integration waits for a tool that may run programs |
| **0.7** ✅ | **Window Designer** (`apps/window-designer`), a NexaOS app written in NexaLang: build a window from headings, text, notes, buttons, a main button, text fields, check boxes, lists, rows and columns; move them; see the preview; get the `ui` program (export or copy). First, in `ui`: `remove`, `clear`, `code`, `save_file`, `open_file`, `to_clipboard`; lists grow with their rows and columns; tests can print a window's widgets | §5: every exported program is accepted by `nexac check` and `fmt --check`, builds with the hardened flags and shows exactly the preview's widgets |
| **0.8** ✅ | **Code Studio** (`apps/code-studio`), a NexaLang code editor written in NexaLang: GtkSourceView with 0.6's colours, mistakes as you type (underlined and listed), Lay out, Explain, Run with Stop. First, in nexac: the `editor` module, `ui.start_program`/`stop_program` with `Output`/`Finished`, `ui.append`; `nexac run` leaves nothing behind when stopped | §5: driven by key presses three ways, packaged, run confined (where Run can't start what it builds: see the decisions) |
| **0.9** ✅ | **Editing NexaLang on Windows**: a VS Code extension (`editors/vscode`: 0.6's grammar and language server, format on save, explanations on hover, as a quick fix and a command), packaged as a `.vsix`; `nexac.exe` for Windows (check, fmt, explain, lsp; build and run say programs are built on NexaOS); `fmt` keeps `\r\n`; modules spelled with the file's capitals everywhere | §5: `nexac.exe` under Wine passes every tool test and gives the same output as the Linux `nexac` on every program; the extension's tests pass with both. Not run on a real Windows PC or VS Code (§6) |
| later | Generics; typed widget handles; threads with message passing; HTTPS (`network` permission); more widgets (images, menus); a hexagon widget; in-place string append; completion in the language server; self-hosting | |
