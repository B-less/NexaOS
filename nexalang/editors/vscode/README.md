# NexaLang for VS Code

NexaLang is the programming language of NexaOS. This extension gives VS Code
(on Windows and Linux):

- **Colours** for `.nx` files.
- **Mistakes as you type**, underlined, with what to do about them.
- **Layout on save**: the file is laid out the one standard way
  (`nexac fmt`). A file with a mistake that stops layout is saved as it is.
- **Explanations**: point at a mistake to see what it means, or use the
  light bulb ("Explain this mistake"), or run **NexaLang: Explain a kind of
  mistake** from the Command Palette (Ctrl+Shift+P).

Everything except the colours comes from `nexac`, NexaLang's compiler, so
VS Code always agrees with it.

## Getting nexac

- **NexaOS / Linux**: `nexac` comes with NexaLang (`cargo build --release`
  in `nexalang/`, then copy `target/release/nexac` to a folder in `PATH`).
- **Windows**: copy `nexac.exe` to a folder of your own (for example
  `C:\Users\you\NexaLang\`) and add that folder to `PATH`, or set
  **nexalang.path** to the full path of `nexac.exe` (or of its folder).

The extension only uses a `nexac` given by its full path: from the
`nexalang.path` setting (only your own settings can set it, never a
folder's), or from a folder of `PATH` that is written as a full path.

## On Windows

`nexac.exe` checks, lays out and explains programs. **Building and running
them happens on NexaOS** (or another Linux): the C a NexaLang program
becomes uses NexaOS's system calls, so `nexac build` and `nexac run` on
Windows say so instead. Copy your `.nx` files to NexaOS and run
`nexac run main.nx` there.

## Settings

| Setting | Meaning |
|---|---|
| `nexalang.path` | Full path to `nexac` (`nexac.exe`), or to the folder that holds it. Empty: found in `PATH`. |

Format on save, 4-space indents and NexaLang as the formatter are on by
default for `.nx` files (`[nexalang]` settings); change them like any other
VS Code setting.
