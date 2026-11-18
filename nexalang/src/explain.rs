//! Offline explanations of nexac's mistakes (0.6): `nexac explain TOPIC`,
//! and the `topic` + `summary` fields of `nexac check --json`.
//!
//! Every message nexac can give belongs to one topic. The topic is found
//! from the message text by `classify` (a fixed list of phrases, checked in
//! order). A test reads every message written in the compiler's source
//! and fails if one has no topic, so a new message can't be forgotten.
//!
//! Each topic carries a small program with the mistake (`wrong`) and the
//! same program fixed (`right`); tests check that nexac rejects the first
//! with this topic and accepts the second.

pub struct Topic {
    pub name: &'static str,
    pub title: &'static str,
    /// One line, for editors and Nexa Mind.
    pub summary: &'static str,
    pub body: &'static str,
    pub wrong: &'static str,
    pub right: &'static str,
}

pub const TOPICS: &[Topic] = &[
    Topic {
        name: "syntax",
        title: "The words or signs aren't in an order NexaLang understands",
        summary: "Something is missing or in the wrong place; look just before the marked spot too.",
        body: "NexaLang reads a program word by word. When it finds something it didn't
expect, it stops and shows where. The real mistake is often just before
the marked spot: a missing `)` or `}`, a missing `=`, or a word from
another language (`&&`, `||`, `;`).

A statement ends at the end of its line, so there's no `;`. Write `and`,
`or` and `not` instead of `&&`, `||` and `!`. Inside `( )` and `[ ]` you
can split a long line wherever you like.",
        wrong: "fn main() {\n    let x = 1 &&\n}\n",
        right: "fn main() {\n    let x = true and false\n    print(x)\n}\n",
    },
    Topic {
        name: "strings",
        title: "Text in quotes",
        summary: "Text goes in \"double quotes\" on one line; {name} puts a value in it, \\{ is a brace.",
        body: "Text is written in double quotes and must end on the same line; use
`\\n` for a line break. `{...}` inside text puts a value there:
\"Hello {name}\". To write a real brace, use `\\{` and `\\}`.

Known escapes: \\n \\t \\r \\0 \\\\ \\\" \\{ \\}.

Only `int`, `float`, `bool` and `str` values can go into text. For a list,
`join` it first: \"{join(names, \", \")}\".",
        wrong: "fn main() {\n    print(\"a {brace\")\n}\n",
        right: "fn main() {\n    print(\"a \\{brace\\}\")\n}\n",
    },
    Topic {
        name: "numbers",
        title: "Numbers",
        summary: "An `int` is 64 bits (about ±9.2 × 10^18); a number can't run into a letter.",
        body: "Whole numbers are `int`s (64 bits: from -9223372036854775808 to
9223372036854775807). `1_000_000` and `0xff` are allowed. Numbers with a
point or an exponent are `float`s (`3.14`, `2e-3`).

A number can't be followed directly by a letter (`3x`): put an operator
between them (`3 * x`).",
        wrong: "fn main() {\n    print(99999999999999999999)\n}\n",
        right: "fn main() {\n    print(9_000_000_000_000_000_000)\n}\n",
    },
    Topic {
        name: "names",
        title: "How names are written",
        summary: "Values and functions start lowercase; structs, enums and variants start with a capital.",
        body: "NexaLang tells values and types apart by their first letter:
- values, parameters, functions and modules start with a lowercase letter
  (`count`, `load_file`);
- `struct`s, `enum`s and their variants start with a capital (`Point`,
  `Shape.Circle`).

Names of built-in functions (`print`, `len`, ...) can't be used for your
own functions, and names starting with `__` are kept for the standard
modules.",
        wrong: "fn main() {\n    let Total = 1\n    print(Total)\n}\n",
        right: "fn main() {\n    let total = 1\n    print(total)\n}\n",
    },
    Topic {
        name: "unknown",
        title: "A name NexaLang doesn't know",
        summary: "Nothing with this name exists here: check the spelling, or define it first.",
        body: "The name isn't a value, function, type, field or variant that exists at
this point. Check the spelling (names are case-sensitive). A value made
inside a block (`if`, `while`, `match` case) only exists inside that
block: make it before the block with `var` if you need it afterwards.

Functions from a module are called through the module: `files.read(p)`,
after `use files` at the top.",
        wrong: "fn main() {\n    let count = 1\n    print(cuont)\n}\n",
        right: "fn main() {\n    let count = 1\n    print(count)\n}\n",
    },
    Topic {
        name: "already-defined",
        title: "Something with this name already exists",
        summary: "Each name can be made only once in a place; pick a new name or change the existing value.",
        body: "NexaLang doesn't let a new value hide an old one with the same name
(\"shadowing\"), because that's an easy way to use the wrong one. Give
the new value its own name, or, if you meant to change the old one,
make it with `var` and assign to it: `total = total + 1`.

The same goes for two functions, fields, variants or cases with the same
name.",
        wrong: "fn main() {\n    let x = 1\n    let x = 2\n    print(x)\n}\n",
        right: "fn main() {\n    var x = 1\n    x = 2\n    print(x)\n}\n",
    },
    Topic {
        name: "let-var",
        title: "Changing something that can't change",
        summary: "Values made with `let` (and parameters) never change; use `var` for ones that do.",
        body: "`let` makes a value that stays the same, `var` one that can change.
Most values never change, so the compiler holds you to it: a value you
didn't mean to change can't change by accident.

Parameters are like `let`. To change a copy, make one: `var list = items`.
Changing a list (`push`, `pop`, `a[i] = ...`) needs it to be `var` too.",
        wrong: "fn main() {\n    let total = 1\n    total += 1\n    print(total)\n}\n",
        right: "fn main() {\n    var total = 1\n    total += 1\n    print(total)\n}\n",
    },
    Topic {
        name: "types",
        title: "A value of the wrong type",
        summary: "This needs a different kind of value; NexaLang never converts silently.",
        body: "Every value has a type (`int`, `float`, `bool`, `str`, a list, a struct,
...), and NexaLang never turns one into another by itself, because
silent conversions lose information unnoticed. Convert on purpose:
- `to_float(n)` and `to_int(x)` between numbers;
- `to_str(x)` or \"{x}\" to make text;
- `parse_int(s)` / `parse_float(s)` to read a number from text (they give
  a `?int` / `?float`, since the text might not be a number).

`if` and `while` need a `bool`: write `if count > 0`, not `if count`.",
        wrong: "fn main() {\n    let n = 2\n    let x = 1.5 + n\n    print(x)\n}\n",
        right: "fn main() {\n    let n = 2\n    let x = 1.5 + to_float(n)\n    print(x)\n}\n",
    },
    Topic {
        name: "match",
        title: "Choosing with `match`",
        summary: "A `match` must handle every case exactly once; `_ =>` handles everything else.",
        body: "`match` looks at a value and runs the first case that fits. It must
cover every possibility, so adding a variant later shows you every place
that needs updating. For numbers and text there are too many values to
list, so add `_ => { }` at the end for \"everything else\".

Each case is listed once, `_` comes last, and a variant's values are
named in the case: `Move(x, y) => { ... }`.",
        wrong: "enum Light {\n    Red\n    Green\n}\n\nfn main() {\n    let l = Light.Red\n    match l {\n        Red => { print(\"stop\") }\n    }\n}\n",
        right: "enum Light {\n    Red\n    Green\n}\n\nfn main() {\n    let l = Light.Red\n    match l {\n        Red => { print(\"stop\") }\n        Green => { print(\"go\") }\n    }\n}\n",
    },
    Topic {
        name: "maybe",
        title: "Values that might be missing (`?T`)",
        summary: "A `?T` is a value or `none`: get the value out with `match` or `unwrap_or`.",
        body: "NexaLang has no null. A value that might be missing has its own type:
`?int` is an `int` or `none`. You can't use it as an `int` until you've
checked:

    match find(text, \"x\") {
        some(i) => { print(i) }
        none => { print(\"not found\") }
    }

or give a default: `unwrap_or(parse_int(s), 0)`. Inside a function that
returns a `?` type, `value?` gives the value or returns `none`.",
        wrong: "fn main() {\n    let n = parse_int(\"12\")\n    print(n + 1)\n}\n",
        right: "fn main() {\n    let n = unwrap_or(parse_int(\"12\"), 0)\n    print(n + 1)\n}\n",
    },
    Topic {
        name: "results",
        title: "Things that can fail (`!T`)",
        summary: "A `!T` is a value or an error: handle it with `match`/`unwrap_or`, or pass it on with `?`.",
        body: "A function that can fail says so in its type: `fn load() -> !str`
returns `ok(text)` or `err(\"why\")`. The caller must deal with the error:
- `match r { ok(v) => { } err(message) => { } }`;
- `unwrap_or(r, default)`;
- or `r?`, which gives the value, or returns the error to its own caller.
  That only works in a function that returns a `!` type itself, such as
  `fn main() -> !void`.",
        wrong: "use files\n\nfn main() {\n    let text = files.read(\"notes.txt\")?\n    print(text)\n}\n",
        right: "use files\n\nfn main() -> !void {\n    let text = files.read(\"notes.txt\")?\n    print(text)\n    return ok()\n}\n",
    },
    Topic {
        name: "returns",
        title: "Returning from a function",
        summary: "A function with `-> type` must return a value of that type on every path.",
        body: "If a function says it returns something (`fn area(r: float) -> float`),
every way through it must end in `return` with a value of that type,
including every `if`/`else` branch and every `match` case. A function
without `-> type` returns nothing, so `return value` isn't allowed there.

A function that calls itself on every path before it can return would
never finish, so it's refused.",
        wrong: "fn sign(n: int) -> int {\n    if n < 0 {\n        return -1\n    }\n}\n\nfn main() {\n    print(sign(3))\n}\n",
        right: "fn sign(n: int) -> int {\n    if n < 0 {\n        return -1\n    }\n    return 1\n}\n\nfn main() {\n    print(sign(3))\n}\n",
    },
    Topic {
        name: "unused",
        title: "Code that does nothing or never runs",
        summary: "This value is never used, or this code can never run; remove it or use it.",
        body: "A line that computes a value and drops it (`x + 1` on its own) is
almost always a mistake: perhaps you meant `x += 1` or `print(x + 1)`.
Code after `return`, `break` or `continue` in the same block can never
run.",
        wrong: "fn main() {\n    var x = 1\n    x + 1\n    print(x)\n}\n",
        right: "fn main() {\n    var x = 1\n    x += 1\n    print(x)\n}\n",
    },
    Topic {
        name: "modules",
        title: "Modules and `use`",
        summary: "Add `use name` at the top; only `pub` items of another file can be used.",
        body: "`use files` (a standard module) or `use helpers` (your helpers.nx, next
to this file) goes at the top of the file. Call its functions through
its name: `helpers.tidy(x)`.

Everything in a module is private unless marked `pub` (`pub fn`,
`pub struct`, `pub enum`). Modules can't use each other in a circle:
move what they share into a third module.

The standard modules are files, time, json, broker, ui and editor.",
        wrong: "fn main() {\n    print(time.now() > 0)\n}\n",
        right: "use time\n\nfn main() {\n    print(time.now() > 0)\n}\n",
    },
    Topic {
        name: "lists",
        title: "Lists, positions and loops",
        summary: "Lists hold one type; positions are `int`s from 0; `for x in list` or `for i in 0..n`.",
        body: "A list holds values of one type: `[1, 2, 3]` is a `[int]`. Positions
start at 0 and are checked when the program runs. An empty list needs
its type: `var names: [str] = []`.

`for item in list { }` goes through a list; `for i in 0..10 { }` counts
from 0 to 9. To go through the letters of text, `split` it first.",
        wrong: "fn main() {\n    let items = []\n    print(len(items))\n}\n",
        right: "fn main() {\n    let items: [int] = []\n    print(len(items))\n}\n",
    },
    Topic {
        name: "main",
        title: "Where the program starts",
        summary: "A program needs `fn main()`, with no parameters, returning nothing or `!void`.",
        body: "Every program starts at `fn main()`. It takes no parameters (read them
with `args()`) and returns nothing, or `!void` so it can use `?`:

    fn main() -> !void {
        ...
        return ok()
    }

An error reaching the end of `main` is printed and the program exits
with status 1.",
        wrong: "fn start() {\n    print(1)\n}\n",
        right: "fn main() {\n    print(1)\n}\n",
    },
    Topic {
        name: "calls",
        title: "Calling a function",
        summary: "Give a function exactly the values it asks for, in order.",
        body: "A function is called with exactly as many values as it has parameters,
each of the right type. `print` takes one value: to print several, put
them in one text: print(\"{a} {b}\").

A variant that holds values is made with them: `Shape.Circle(2.0)`; one
that holds nothing is written without `()`: `Light.Red`.",
        wrong: "fn add(a: int, b: int) -> int {\n    return a + b\n}\n\nfn main() {\n    print(add(1))\n}\n",
        right: "fn add(a: int, b: int) -> int {\n    return a + b\n}\n\nfn main() {\n    print(add(1, 2))\n}\n",
    },
    Topic {
        name: "loops",
        title: "`break` and `continue`",
        summary: "`break` and `continue` only work inside `while` or `for`.",
        body: "`break` leaves the loop it's in; `continue` goes on to the next round.
Outside a loop there's nothing to leave: use `return` to leave a
function.",
        wrong: "fn main() {\n    break\n}\n",
        right: "fn main() {\n    while true {\n        break\n    }\n}\n",
    },
    Topic {
        name: "limits",
        title: "Something is too big or too deep",
        summary: "This goes past one of NexaLang's limits; split it into smaller pieces.",
        body: "To keep the compiler safe from hostile files, nesting is limited to 200
levels in one expression or block, strings inside strings to 8 levels,
and a file to 999,999 lines. Put inner parts in variables or functions,
or split the file into modules.",
        wrong: "fn main() {\n    print(\"{\"{\"{\"{\"{\"{\"{\"{\"{\"1}\"}\"}\"}\"}\"}\"}\"}\"}\")\n}\n",
        right: "fn main() {\n    let inner = \"{1}\"\n    print(\"{inner}\")\n}\n",
    },
    Topic {
        name: "files",
        title: "A source file that can't be read",
        summary: "nexac couldn't read this file, or it isn't UTF-8 text.",
        body: "NexaLang source files are UTF-8 text ending in `.nx`. Check that the
file exists, that you may read it, and that your editor saved it as
UTF-8.",
        wrong: "",
        right: "",
    },
];

/// The topic used when no phrase fits (a test keeps this from happening
/// for every message in the compiler).
pub const OTHER: Topic = Topic {
    name: "other",
    title: "A mistake",
    summary: "Read the message and the hint; `nexac explain` lists every topic.",
    body: "This mistake has no longer explanation yet. The message and its hint
say what nexac expected.",
    wrong: "",
    right: "",
};

/// Phrases, checked in order: the first one found in the message decides
/// its topic. More specific phrases come first.
const PHRASES: &[(&str, &str)] = &[
    // files and limits
    ("isn't valid UTF-8 text", "files"),
    ("can't read ", "files"),
    ("end in .nx", "files"),
    ("more than", "limits"),
    ("nested too deeply", "limits"),
    ("too many strings inside strings", "limits"),
    // main
    ("`main`", "main"),
    ("fn main()", "main"),
    // loops
    ("`break` and `continue`", "loops"),
    // changing fixed values
    ("made with `let`", "let-var"),
    ("can't {} this", "let-var"),
    // as given: "can't change this", "can't `push` to this"
    ("can't change this", "let-var"),
    ("` to this", "let-var"),
    // unused / unreachable
    ("can never run", "unused"),
    ("never used", "unused"),
    // names
    ("start with a lowercase", "names"),
    ("start with a capital", "names"),
    ("is a built-in function; pick another name", "names"),
    ("are reserved for the standard modules", "names"),
    ("is for the standard modules only", "names"),
    ("also defined in", "already-defined"),
    // modules
    ("module", "modules"),
    ("`use`", "modules"),
    ("is private", "modules"),
    ("which is private", "modules"),
    // match
    ("is already handled above", "match"),
    ("match", "match"),
    ("this case", "match"),
    ("the case names", "match"),
    ("a case can't contain", "strings"),
    // maybe and results
    ("`none`", "maybe"),
    ("write `some(name)`", "maybe"),
    ("to get the value out", "match"),
    ("already a `?` value", "maybe"),
    ("`?` passes", "results"),
    ("`?` works on", "results"),
    ("`ok()`", "results"),
    ("`unwrap_or`", "results"),
    ("an error is described", "results"),
    ("this result holds no value", "results"),
    ("type this `err` is", "results"),
    // returns
    ("always calls itself", "returns"),
    ("without returning", "returns"),
    ("return", "returns"),
    // duplicates
    ("already exists", "already-defined"),
    ("defined twice", "already-defined"),
    ("appears twice", "already-defined"),
    ("given twice", "already-defined"),
    ("is already used above", "already-defined"),
    // unknown
    ("unknown name", "unknown"),
    ("unknown function", "unknown"),
    ("unknown type", "unknown"),
    ("unknown enum", "unknown"),
    ("unknown struct", "unknown"),
    ("is a struct, not an enum", "types"),
    ("is an enum; make one", "types"),
    ("has no field", "unknown"),
    ("has no variant", "unknown"),
    ("has no function", "unknown"),
    ("has no fields", "unknown"),
    ("missing field", "calls"),
    // calls
    ("value(s)", "calls"),
    ("`print` takes", "calls"),
    ("holds no values", "calls"),
    ("empty `()` after a variant", "calls"),
    ("only functions can be called", "calls"),
    // lists
    ("empty list", "lists"),
    ("loop over", "lists"),
    ("a range needs", "lists"),
    ("list position", "lists"),
    ("index", "lists"),
    ("every item in a list", "lists"),
    ("list", "lists"),
    // strings
    ("string", "strings"),
    ("escape", "strings"),
    ("turned into text", "strings"),
    // numbers
    ("number", "numbers"),
    ("`0x`", "numbers"),
    // syntax (before types: "expected X, found Y" is the parser's)
    ("unexpected", "syntax"),
    (", found ", "syntax"),
    ("comparisons can't be chained", "syntax"),
    // a value that might be missing, or might be an error, used as if it
    // were there
    ("`??T`", "types"),
    ("`?void`", "types"),
    ("can't be stacked", "types"),
    ("`?", "maybe"),
    ("`!", "results"),
    // types
    ("contains itself", "types"),
    ("void", "types"),
    ("isn't a type", "types"),
    ("can't be stacked", "types"),
    ("`??T`", "types"),
    ("can't tell what type", "types"),
    ("doesn't produce a value", "types"),
    ("but this is", "types"),
    ("can't add", "types"),
    ("can't compare", "types"),
    ("can't use", "types"),
    ("can't be used on", "types"),
    ("can't make a", "types"),
    ("this is a `", "types"),
    ("needs a", "types"),
    ("expected `", "types"),
];

pub fn find(name: &str) -> Option<&'static Topic> {
    TOPICS.iter().find(|t| t.name == name)
}

/// The topic of a message nexac gave (or `OTHER`).
pub fn classify(msg: &str) -> &'static Topic {
    for (phrase, topic) in PHRASES {
        if msg.contains(phrase) {
            return find(topic).unwrap_or(&OTHER);
        }
    }
    &OTHER
}

/// `nexac explain` with no topic: the list.
pub fn list() -> String {
    let mut s = String::from("nexac explain TOPIC   (or: nexac explain \"an error message\")\n\nTopics:\n");
    let w = TOPICS.iter().map(|t| t.name.len()).max().unwrap_or(0);
    for t in TOPICS {
        s += &format!("  {:w$}  {}\n", t.name, t.title, w = w);
    }
    s
}

/// The full explanation of one topic, as plain text.
pub fn render(t: &Topic) -> String {
    let mut s = format!("{} ({})\n\n{}\n\n{}\n", t.title, t.name, t.summary, t.body);
    if !t.wrong.is_empty() {
        s += &format!("\nFor example, this is refused:\n\n{}\nand this works:\n\n{}", indent(t.wrong), indent(t.right));
    }
    s
}

fn indent(code: &str) -> String {
    code.lines().map(|l| if l.is_empty() { String::from("\n") } else { format!("    {}\n", l) }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every message written in the compiler's source, with `{}` where a
    /// value goes in: the first string literal after `Diag::new(...,`,
    /// `self.err(...,` or `err(...,` / `general(...,`.
    fn messages() -> Vec<String> {
        let srcs = [
            include_str!("lexer.rs"),
            include_str!("parser.rs"),
            include_str!("checker.rs"),
            include_str!("main.rs"),
        ];
        let mut out = Vec::new();
        // the first literal of each call that makes a message
        let first_literal = |rest: &str| -> Option<String> {
            let call_end = rest.find(";\n").unwrap_or(rest.len()).min(400);
            let call = &rest[..call_end];
            let q = call.find('"')?;
            Some(literal_at(&call[q..]))
        };
        for src in srcs {
            for start in ["Diag::new(", "self.err(", "c.err(", "fail(", "Report::general("] {
                for (i, _) in src.match_indices(start) {
                    out.extend(first_literal(&src[i..]));
                }
            }
            // `let msg = if .. { format!("a") } else { format!("b") };`
            for (i, _) in src.match_indices("let msg = ") {
                let rest = &src[i..];
                let end = rest.find("msg)").unwrap_or(0);
                for (j, _) in rest[..end].match_indices("format!(\"") {
                    out.push(literal_at(&rest[j + 8..]));
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// The text of the string literal starting at `s` (at its `"`).
    fn literal_at(s: &str) -> String {
        let b = s.as_bytes();
        let mut end = 1;
        while end < b.len() && !(b[end] == b'"' && b[end - 1] != b'\\') {
            end += 1;
        }
        s[1..end].to_string()
    }

    #[test]
    fn every_message_in_the_compiler_has_a_topic() {
        let ms = messages();
        assert!(ms.len() > 100, "found only {} messages", ms.len());
        let missing: Vec<&String> = ms.iter().filter(|m| classify(m).name == "other").collect();
        assert!(missing.is_empty(), "messages without a topic: {:#?}", missing);
    }

    #[test]
    fn topics_are_unique_and_phrases_name_real_topics() {
        for (i, t) in TOPICS.iter().enumerate() {
            assert!(TOPICS[i + 1..].iter().all(|u| u.name != t.name), "{} twice", t.name);
            assert!(!t.summary.contains('\n'), "{}: summary must be one line", t.name);
        }
        for (_, topic) in PHRASES {
            assert!(find(topic).is_some(), "phrase names unknown topic {}", topic);
        }
    }

    #[test]
    fn known_messages_go_to_the_right_topic() {
        let cases = [
            ("unknown name `cuont`", "unknown"),
            ("can't change `x`: it was made with `let` (or is a parameter)", "let-var"),
            ("expected `int`, but this is `str`", "types"),
            ("this `match` doesn't handle Green", "match"),
            ("expected `)`, found end of line", "syntax"),
            ("module `time` isn't used here", "modules"),
            ("`?` passes an error on to the caller, so this function must return `!` type", "results"),
            ("this string never ends", "strings"),
            ("can't change this", "let-var"),
            ("can't `push` to this", "let-var"),
            ("can't use `+` on `?int` and `int`", "maybe"),
            ("expected `int`, but this is `!int`", "results"),
            ("`main` takes no parameters and returns nothing (or `!void`)", "main"),
        ];
        for (m, want) in cases {
            assert_eq!(classify(m).name, want, "{}", m);
        }
    }
}
