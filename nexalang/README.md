# NexaLang: the programming language of NexaOS

Started 2026-11-16. Current version **0.5** (compiler `nexac`, in `nexalang/`).

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
| **Passwords are asked for by `ui`, checked by the broker** (0.4) | `ui.broker_call` shows a password dialog when the broker asks, three tries, the way `nexa_common/broker.py` does; the password goes only into the request to the broker. Apps never check or keep it | Apps asking with their own dialogs |

## 2. The language (v0.5)

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

**Windows: `ui` (0.4).** GTK 3 with Hexa built in. Every function that
makes a widget takes its parent (a column or a row) and returns the new
widget's number.

| Group | Functions |
|---|---|
| Window | `start(app_id, title) -> !int` (opens the window, returns its main column; `err` when there's no display; one window per program) |
| Layout | `column(parent)`, `row(parent)` (text fields and columns in a row share the free width) |
| Text | `label`, `heading`, `note` (small, quiet), `mono` (JetBrains Mono: numbers that update), `display` (one big number: JetBrains Mono, 22 pt; 0.5) — all `(parent, text) -> int`; long text wraps |
| Controls | `button(parent, text)`, `primary_button` (one per window, accent colour), `danger_button`, `entry(parent, hint)`, `check(parent, text, on)`, `list(parent, empty_text)` (scrolling rows of text) |
| Changing | `set_text(id, text)`, `get_text(id)`, `set_checked`, `is_checked`, `set_items(list, [str])` (clears the selection), `selected(list) -> ?int`, `select(list, i)`, `select_none`, `set_enabled`, `set_visible`, `focus`, `tooltip(id, text)`, `shortcut(button, "Ctrl+N")` (shown in the tooltip), `status(text)`, `show_error(text)` (the line at the bottom) |
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
}
```

Keyboard: Tab moves between widgets (and out of a list; arrow keys move
inside it), Space and Enter press, Esc and Ctrl+Q close. `Closed` doesn't
close the window by itself: the program decides (returning from `main`
does). Windows are 560×640 to start and work down to 360 px wide.

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
window.

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
  src/std/*.nx     the standard modules, built into nexac
  examples/*.nx    + .out expected output (+ .in input, .args arguments)
  tests/fail/*.nx  programs that must be rejected (53)
  tests/trap/*.nx  programs that must stop with a runtime error (11)
  tests/modules/   a three-file program, and 11 module mistakes
  tests/broker/    a stand-in broker (Python) and a client
  tests/json/      JSON round-trip reader + differential fuzz vs Python
  tests/stress/    2-million-round memory churn
  tests/ui/        window programs driven by real key presses on a
                   headless X server (drive.py: Xvfb + xdotool), *.steps
                   with exact *.out; tests/ui/trap: misuse (14)
  tests/fuzz/      the compiler fuzzer (fuzz.py)
  examples/ui/     window examples
../apps/*/src/main.nx   NexaOS apps written in NexaLang (0.5); their
                 tests/*.steps run with the window tests, three ways
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

## 6. Known limits of 0.5

- **One window per program**, and widgets can't be removed (hide them
  with `set_visible`, or use a list for things that come and go).
- **List rows are text only**; no images, menus, tabs, sliders, file
  chooser or drag and drop yet. **No file chooser yet**, so a NexaLang app
  can't open the person's files the way WRITING_NEXA_APPS.md §3 asks
  (0.4 planned it for 0.5; it was moved to later, because 0.5's sample app
  doesn't open files and an untested file chooser is worse than none).
  Until then, write apps that keep their own data, like Focus Timer.
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
| **0.6** | `nexac fmt`, syntax highlighting, language server; Nexa Mind explains compiler errors offline | |
| later | Generics; typed widget handles; threads with message passing; HTTPS (`network` permission); more widgets (file chooser first, then images, menus); a hexagon widget; in-place string append; self-hosting | |
