# Editing NexaLang

Everything here comes from `nexac` itself, so editors always agree with the
compiler (0.6).

| What | How |
|---|---|
| Mistakes, as JSON | `nexac check --json app.nx` (or `--stdin app.nx` for unsaved text; the modules next to `app.nx` are read from the disk) |
| Layout | `nexac fmt app.nx` (in place), `nexac fmt --check app.nx`, `nexac fmt --stdin` |
| What a mistake means | `nexac explain` (topics), `nexac explain results`, `nexac explain "the message"`, `--json` for programs |
| Language server | `nexac lsp` (stdin/stdout): mistakes as you type, formatting, an explanation on hover |
| Colours, GtkSourceView 4 | `gtksourceview/nexalang.lang` (NexaOS's text editor has it: `scripts/stage-text-editor.sh` installs it to `/usr/share/gtksourceview-4/language-specs/`) |
| NexaOS's NexaLang editor | `apps/code-studio` (0.8): GtkSourceView with these colours (built in through the `editor` module, with `gtksourceview/nexa-horizon.xml` as the fallback for the theme's `nexa.xml`), mistakes from `check --json` as you type, `fmt --stdin`, `explain --json`, `run` |
| Colours, VS Code and TextMate editors | `vscode/syntaxes/nexalang.tmLanguage.json`, `vscode/language-configuration.json` (the VS Code extension itself is NexaLang 0.9) |

## `nexac check --json`

One line of JSON on stdout; exit status 0 (no mistakes), 1 (mistakes) or
2 (wrong command line).

```json
{"nexac":"0.6.0","file":"bad_type.nx","ok":false,"diagnostics":[
  {"file":"bad_type.nx","line":2,"column":9,"length":6,
   "message":"unknown type `string`","hint":"the string type is `str`",
   "topic":"unknown","summary":"Nothing with this name exists here: check the spelling, or define it first."}]}
```

- `line` and `column` start at 1; `column` counts characters (Unicode
  letters, a tab is one), not bytes. `line` 0 means the mistake isn't
  about one place (a file that can't be read).
- `length`: how many characters to underline (the word there, else 1).
- `file`: the file the mistake is in (a module's file, relative to the
  main file's folder, or `std/ui.nx` for a standard module).
- `hint` may be `null`. `topic` is a name for `nexac explain`.

## The language server

Start `nexac lsp` with no arguments; it speaks the Language Server
Protocol over stdin/stdout (`Content-Length` framing). It offers:
`textDocument/publishDiagnostics` after open, change and save (full-text
sync), `textDocument/formatting` and `textDocument/hover` (on a mistake).
Positions are UTF-16 units, as the protocol says.

Other editors, for example Helix (`languages.toml`):

```toml
[[language]]
name = "nexalang"
scope = "source.nexalang"
file-types = ["nx"]
comment-token = "//"
indent = { tab-width = 4, unit = "    " }
language-servers = ["nexac"]

[language-server.nexac]
command = "nexac"
args = ["lsp"]
```

## Tests

`cd nexalang && cargo test` checks both colour files against
`tests/highlight/sample.expected` (GtkSourceView through a small C
program; the TextMate grammar through `vscode-textmate`, VS Code's own
engine: run `npm ci` in `tests/highlight` once), and runs whole language
server sessions (`tests/tooling.rs`).
