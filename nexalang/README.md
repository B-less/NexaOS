# NexaLang: the programming language of NexaOS

Started 2026-11-16. Current version **0.3** (compiler `nexac`, in `nexalang/`).

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

## 2. The language (v0.3)

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
| `files` | `read(path) -> !str`, `read_lines -> ![str]`, `write(path, text) -> !void` (all-or-nothing), `append`, `exists`, `is_dir`, `list(dir) -> ![str]` (sorted names), `make_dir` (with parents), `remove` (a file or an *empty* folder; never contents), `join_path(a, b)`, `name(path)`, `home()`, `config_dir(id)`, `data_dir(id)`, `cache_dir(id)` (the app's XDG folders; ids follow nexa.toml's rules) |
| `time` | `now() -> int` (Unix seconds), `millis()` (for measuring), `sleep_ms(n)`, `format(t, "%Y-%m-%d")`, `format_utc`, `iso(t)` |
| `json` | `parse(text) -> !Json` (errors give line and column; 200 levels max), `to_text(j)`, `pretty(j)`, `get(j, key) -> ?Json`, `at(j, i)`, `as_text`, `as_num`, `as_int`, `as_bool`, `items`, `keys`, `obj(fields)`, `field(key, value)`, `set(j, key, value)`. Values: `Json.Null`, `Json.Bool(b)`, `Json.Num(n)`, `Json.Text(s)`, `Json.List(items)`, `Json.Obj(fields)` |
| `broker` | `call(action, args: Json) -> !Json` (one request to `/run/nexa-broker.sock`, the reply as JSON; 30-minute limit), `call_timeout(action, args, ms)`, `no_args()` |

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
```

All identifiers are prefixed in C (`nx_v_`, `nx_fn_`, `nx_S_`, `nx_E_`,
`nx_M_`, `nx_R_`, `nx_f_`), so a NexaLang name can never collide with C;
module functions get their module's name with its length
(`nx_fn_4json_parse`). String literals are emitted with octal escapes
only. A runtime error knows its file: the location passed to the runtime
is file number × 1,000,000 + line.

## 5. Testing done

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

## 6. Known limits of 0.3

- **Single-threaded.** Reference counts aren't atomic; threads (later)
  will use message passing, never shared values.
- No generic functions or types of your own (only the built-in `?T`,
  `!T` and `[T]`).
- `s += "x"` in a loop copies the text each time; collect pieces in a list
  and `join` them.
- No network access yet (only the broker's local socket), and no GUI.
- The broker client sends requests and returns the reply as it is. How
  the broker asks for a password isn't written down in SYSTEM_BROKER.md,
  so password prompts aren't handled by `broker` yet.
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
| **0.4** | **GTK bindings with Hexa built in**: `nexa.ui` windows, buttons, lists that use the `@nexa_*` theme colours and fonts automatically | A NexaLang app passes WRITING_NEXA_APPS.md §6 |
| **0.5** | `nexa-pack new my-app --lang nexa`; `.nexa` packages built from `.nx` | An app in the Store written in NexaLang |
| **0.6** | `nexac fmt`, syntax highlighting, language server; Nexa Mind explains compiler errors offline | |
| later | Generics; threads with message passing; HTTPS (`network` permission); broker password prompts; in-place string append; self-hosting | |
