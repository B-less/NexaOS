# Changes

## 0.9.2

- The language server starts in a real VS Code: the client logs with
  `.error()`/`.info()`, which only a log output channel has; 0.9.0 and
  0.9.1 made a plain one, so any message from the client stopped it with
  "this.outputChannel.error is not a function". The tests' stand-in client now
  writes to the channel like the real one.

## 0.9.1

- `nexalang.path` can be the folder that holds `nexac.exe` (or `nexac`), not
  only the program itself, and quotes around it (Windows' "Copy as path")
  are fine. Found on the first try on a real Windows PC.

## 0.9.0

The first version (NexaLang 0.9): colours, mistakes as you type, layout on
save, explanations (on hover, as a quick fix, and the "NexaLang: Explain a
kind of mistake" command), all from `nexac`.
