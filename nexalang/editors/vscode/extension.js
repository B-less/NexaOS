// NexaLang for VS Code (NexaLang 0.9).
//
// Colours come from the TextMate grammar (syntaxes/). Everything else comes
// from nexac itself, so VS Code always agrees with the compiler:
//   - `nexac lsp`: mistakes as you type, layout (format on save), and an
//     explanation when the pointer is over a mistake;
//   - `nexac explain --json`: the "Explain" command and the quick fix on
//     each mistake.
// nexac is always started by its full path with an argument list, never
// through a shell (lib.js).
'use strict';

const os = require('os');
const vscode = require('vscode');
const { LanguageClient } = require('vscode-languageclient/node');
const lib = require('./lib');

const SCHEME = 'nexalang-explain';

let client = null;
let output = null;
let warnedAbout = '';
const pages = new Map(); // explanation pages shown: uri -> Markdown

function settingPath() {
  // `machine` scope (package.json): only the person's own settings can
  // name a program to run, never a folder's .vscode/settings.json
  return vscode.workspace.getConfiguration('nexalang').get('path', '');
}

function locate() {
  return lib.findNexac({ setting: settingPath(), envPath: process.env.PATH, platform: process.platform });
}

async function warn(message) {
  if (warnedAbout === message) return; // once, not on every file
  warnedAbout = message;
  const choice = await vscode.window.showWarningMessage(`NexaLang: ${message}`, 'Set the path');
  if (choice === 'Set the path') {
    await vscode.commands.executeCommand('workbench.action.openSettings', 'nexalang.path');
  }
}

async function startClient() {
  const found = locate();
  if (found.error) {
    output.appendLine(found.error);
    warn(found.error);
    return;
  }
  output.appendLine(`Using ${found.path}`);
  // No `transport`: with TransportKind.stdio the client would add
  // `--stdio` to the arguments. Without one it still talks over
  // stdin/stdout, and nexac gets exactly `lsp`.
  const run = lib.serverCommand(found.path, os.homedir());
  client = new LanguageClient(
    'nexalang',
    'NexaLang',
    { run, debug: run },
    {
      documentSelector: [
        { scheme: 'file', language: 'nexalang' },
        { scheme: 'untitled', language: 'nexalang' },
      ],
      outputChannel: output,
    }
  );
  try {
    await client.start();
  } catch (e) {
    const message = `nexac's language server didn't start: ${e && e.message ? e.message : e} (details: View, Output, NexaLang)`;
    output.appendLine(message);
    warn(message);
    client = null;
  }
}

async function stopClient() {
  const c = client;
  client = null;
  if (c) {
    try {
      await c.stop();
    } catch (e) {
      output.appendLine(`Stopping the language server: ${e && e.message ? e.message : e}`);
    }
  }
}

async function restart() {
  warnedAbout = '';
  await stopClient();
  await startClient();
}

/** Show one topic's explanation beside the code (a read-only page). */
async function showTopic(t) {
  const uri = vscode.Uri.from({ scheme: SCHEME, path: `/${t.topic}.md` });
  pages.set(uri.toString(), lib.topicMarkdown(t));
  const doc = await vscode.workspace.openTextDocument(uri);
  await vscode.languages.setTextDocumentLanguage(doc, 'markdown');
  await vscode.window.showTextDocument(doc, { viewColumn: vscode.ViewColumn.Beside, preview: true, preserveFocus: false });
}

/** "NexaLang: Explain a kind of mistake" (with a topic: from a quick fix). */
async function explain(topic) {
  const found = locate();
  if (found.error) {
    warn(found.error);
    return;
  }
  try {
    const topics = await lib.runExplain(found.path, typeof topic === 'string' ? topic : '', os.homedir());
    if (typeof topic === 'string' && topic !== '') {
      await showTopic(topics[0]);
      return;
    }
    const pick = await vscode.window.showQuickPick(
      topics.map((t) => ({ label: t.title, description: t.topic, detail: t.summary, topic: t })),
      { placeHolder: 'Which kind of mistake?', matchOnDescription: true, matchOnDetail: true }
    );
    if (pick) await showTopic(pick.topic);
  } catch (e) {
    vscode.window.showErrorMessage(`NexaLang: ${e.message}`);
  }
}

/** A quick fix on each of nexac's mistakes: "Explain: <topic>". */
const explainActions = {
  provideCodeActions(_document, _range, context) {
    const seen = new Set();
    const actions = [];
    for (const d of context.diagnostics) {
      const topic = lib.diagnosticTopic(d);
      if (!topic || seen.has(topic)) continue;
      seen.add(topic);
      const a = new vscode.CodeAction(`Explain this mistake (${topic})`, vscode.CodeActionKind.QuickFix);
      a.command = { command: 'nexalang.explain', title: 'Explain', arguments: [topic] };
      a.diagnostics = [d];
      actions.push(a);
    }
    return actions;
  },
};

function activate(context) {
  // a log channel: vscode-languageclient writes to it with .error(),
  // .warn() and .info(), which a plain output channel doesn't have (0.9.2:
  // found on the first run in a real VS Code)
  output = vscode.window.createOutputChannel('NexaLang', { log: true });
  context.subscriptions.push(
    output,
    vscode.workspace.registerTextDocumentContentProvider(SCHEME, {
      provideTextDocumentContent: (uri) => pages.get(uri.toString()) || '',
    }),
    vscode.commands.registerCommand('nexalang.explain', explain),
    vscode.commands.registerCommand('nexalang.restart', restart),
    vscode.languages.registerCodeActionsProvider({ language: 'nexalang' }, explainActions, {
      providedCodeActionKinds: [vscode.CodeActionKind.QuickFix],
    }),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration('nexalang.path')) restart();
    })
  );
  return startClient();
}

function deactivate() {
  return stopClient();
}

module.exports = { activate, deactivate };
