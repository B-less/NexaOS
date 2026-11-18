// extension.js with a stand-in for VS Code's API and for the language
// client (VS Code itself can't be downloaded on the machine that tests
// this; see NEXALANG.md). It checks what the extension asks VS Code to do:
// which commands and providers it registers, how it starts nexac, what it
// says when nexac is missing, and the "Explain" command with a real nexac.
'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const Module = require('module');
const fs = require('fs');
const os = require('os');
const path = require('path');

const NEXAC = path.resolve(process.env.NEXAC || path.join(__dirname, '..', '..', '..', 'target', 'release', 'nexac'));

function fakeVscode(settings) {
  const log = { commands: {}, providers: [], warnings: [], errors: [], shown: [], clients: [], picks: [], config: [], channels: [], output: [] };
  const vscode = {
    log,
    ViewColumn: { Beside: -2 },
    CodeActionKind: { QuickFix: 'quickfix' },
    CodeAction: class {
      constructor(title, kind) {
        this.title = title;
        this.kind = kind;
      }
    },
    Uri: { from: (o) => ({ scheme: o.scheme, path: o.path, toString: () => `${o.scheme}:${o.path}` }) },
    window: {
      // like VS Code: only a channel made with { log: true } has .error() etc.
      createOutputChannel: (name, opts) => {
        log.channels.push([name, opts]);
        const c = { appendLine: (m) => log.output.push(m), dispose: () => {} };
        if (opts && opts.log === true) {
          for (const k of ['error', 'warn', 'info', 'debug', 'trace']) c[k] = (m) => log.output.push(`${k}: ${m}`);
        }
        return c;
      },
      showWarningMessage: async (m, ...buttons) => {
        log.warnings.push([m, buttons]);
        return undefined;
      },
      showErrorMessage: async (m) => log.errors.push(m),
      showQuickPick: async (items) => {
        log.picks.push(items);
        return items.find((i) => i.description === 'maybe');
      },
      showTextDocument: async (doc, opts) => log.shown.push([doc, opts]),
    },
    workspace: {
      getConfiguration: (section) => ({ get: (k, d) => (k in settings ? settings[k] : d), section }),
      registerTextDocumentContentProvider: (scheme, p) => {
        log.providers.push(['content', scheme, p]);
        return { dispose() {} };
      },
      onDidChangeConfiguration: (f) => {
        log.config.push(f);
        return { dispose() {} };
      },
      openTextDocument: async (uri) => {
        const content = log.providers.find((p) => p[0] === 'content')[2].provideTextDocumentContent(uri);
        return { uri, content, languageId: 'plaintext' };
      },
    },
    languages: {
      registerCodeActionsProvider: (selector, p, meta) => {
        log.providers.push(['actions', selector, p, meta]);
        return { dispose() {} };
      },
      setTextDocumentLanguage: async (doc, lang) => {
        doc.languageId = lang;
        return doc;
      },
    },
    commands: {
      registerCommand: (name, f) => {
        log.commands[name] = f;
        return { dispose() {} };
      },
      executeCommand: async () => {},
    },
  };
  const client = {
    LanguageClient: class {
      constructor(id, name, server, options) {
        Object.assign(this, { id, name, server, options, started: 0, stopped: 0 });
        log.clients.push(this);
      }
      async start() {
        this.started++;
        // the real client logs with these (an error, a warning, information)
        this.options.outputChannel.error('test error');
        this.options.outputChannel.warn('test warning');
        this.options.outputChannel.info('test info');
      }
      async stop() {
        this.stopped++;
      }
    },
  };
  return { vscode, client };
}

/** Load a fresh extension.js against the stand-ins. */
function load(settings, envPath) {
  const { vscode, client } = fakeVscode(settings);
  const resolve = Module._resolveFilename;
  Module._resolveFilename = function (req, ...rest) {
    if (req === 'vscode' || req === 'vscode-languageclient/node') return req;
    return resolve.call(this, req, ...rest);
  };
  require.cache.vscode = { id: 'vscode', filename: 'vscode', loaded: true, exports: vscode };
  require.cache['vscode-languageclient/node'] = { id: 'lc', filename: 'lc', loaded: true, exports: client };
  const file = require.resolve('../extension.js');
  delete require.cache[file];
  const oldPath = process.env.PATH;
  process.env.PATH = envPath;
  try {
    return { ext: require(file), vscode, restore: () => ((process.env.PATH = oldPath), (Module._resolveFilename = resolve)) };
  } finally {
    process.env.PATH = oldPath;
  }
}

test('activation registers the commands and providers, and starts `nexac lsp`', async (t) => {
  // a folder with the nexac under test called `nexac` (it may be a script
  // running nexac.exe under Wine)
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'nexalang-path-'));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  fs.symlinkSync(NEXAC, path.join(dir, 'nexac'));
  const found = path.join(dir, 'nexac');
  const { ext, vscode, restore } = load({}, `.:${dir}`);
  const ctx = { subscriptions: [] };
  const oldPath = process.env.PATH;
  process.env.PATH = `.:${dir}`;
  try {
    await ext.activate(ctx);
  } finally {
    process.env.PATH = oldPath;
  }
  const log = vscode.log;
  assert.deepEqual(Object.keys(log.commands).sort(), ['nexalang.explain', 'nexalang.restart']);
  assert.equal(ctx.subscriptions.length, 6);
  assert.equal(log.clients.length, 1);
  const c = log.clients[0];
  assert.equal(c.started, 1);
  assert.deepEqual(c.server.run, { command: found, args: ['lsp'], options: { cwd: os.homedir(), shell: false, windowsHide: true } });
  assert.equal(c.server.run.transport, undefined); // else the client adds --stdio
  assert.deepEqual(c.options.documentSelector, [
    { scheme: 'file', language: 'nexalang' },
    { scheme: 'untitled', language: 'nexalang' },
  ]);
  assert.deepEqual(log.warnings, []);
  assert.deepEqual(log.channels, [['NexaLang', { log: true }]]);
  assert.ok(log.output.includes('error: test error'), JSON.stringify(log.output));

  // the quick fix: one "Explain" per topic of nexac's own mistakes
  const actions = log.providers.find((p) => p[0] === 'actions');
  assert.deepEqual(actions[1], { language: 'nexalang' });
  const diags = [
    { source: 'nexac', code: 'maybe', message: 'a' },
    { source: 'nexac', code: 'maybe', message: 'b' },
    { source: 'other', code: 'x', message: 'c' },
    { source: 'nexac', code: 'let-var', message: 'd' },
  ];
  const got = actions[2].provideCodeActions(null, null, { diagnostics: diags });
  assert.deepEqual(
    got.map((a) => [a.title, a.kind, a.command.command, a.command.arguments]),
    [
      ['Explain this mistake (maybe)', 'quickfix', 'nexalang.explain', ['maybe']],
      ['Explain this mistake (let-var)', 'quickfix', 'nexalang.explain', ['let-var']],
    ]
  );

  // "Explain" from the quick fix, with the real nexac: shown beside, as Markdown
  process.env.PATH = `.:${dir}`;
  try {
    await log.commands['nexalang.explain']('let-var');
    await log.commands['nexalang.explain'](); // from the Command Palette: pick one
  } finally {
    process.env.PATH = oldPath;
  }
  assert.deepEqual(log.errors, []);
  assert.equal(log.shown.length, 2);
  const [doc, opts] = log.shown[0];
  assert.equal(doc.languageId, 'markdown');
  assert.equal(doc.uri.toString(), 'nexalang-explain:/let-var.md');
  assert.match(doc.content, /^# .+\n/);
  assert.match(doc.content, /## Refused\n\n```nexalang\n/);
  assert.deepEqual(opts, { viewColumn: -2, preview: true, preserveFocus: false });
  assert.ok(log.picks[0].length >= 20);
  assert.equal(log.shown[1][0].uri.toString(), 'nexalang-explain:/maybe.md');

  // changing nexalang.path restarts the server
  log.config[0]({ affectsConfiguration: (k) => k === 'nexalang.path' });
  await new Promise((r) => setTimeout(r, 50));
  assert.equal(c.stopped, 1);
  await ext.deactivate();
  restore();
});

test('without nexac: one warning that says how to fix it, and no server', async () => {
  const { ext, vscode, restore } = load({}, '/nonexistent-folder');
  const oldPath = process.env.PATH;
  process.env.PATH = '/nonexistent-folder';
  try {
    await ext.activate({ subscriptions: [] });
    await vscode.log.commands['nexalang.explain']('maybe');
  } finally {
    process.env.PATH = oldPath;
  }
  assert.equal(vscode.log.clients.length, 0);
  assert.equal(vscode.log.warnings.length, 1, JSON.stringify(vscode.log.warnings));
  assert.match(vscode.log.warnings[0][0], /^NexaLang: Can't find nexac/);
  assert.deepEqual(vscode.log.warnings[0][1], ['Set the path']);
  assert.equal(vscode.log.shown.length, 0);
  await ext.deactivate();
  restore();
});

test('a relative nexalang.path is refused, not run', async () => {
  const { ext, vscode, restore } = load({ path: 'nexac' }, path.dirname(NEXAC));
  await ext.activate({ subscriptions: [] });
  assert.equal(vscode.log.clients.length, 0);
  assert.match(vscode.log.warnings[0][0], /must be the full path/);
  await ext.deactivate();
  restore();
});

test('the package: what VS Code reads from package.json', () => {
  const pkg = JSON.parse(fs.readFileSync(path.join(__dirname, '..', 'package.json'), 'utf8'));
  assert.equal(pkg.contributes.configuration.properties['nexalang.path'].scope, 'machine');
  assert.equal(pkg.contributes.configurationDefaults['[nexalang]']['editor.defaultFormatter'], `${pkg.publisher}.${pkg.name}`);
  assert.equal(pkg.contributes.configurationDefaults['[nexalang]']['editor.formatOnSave'], true);
  assert.deepEqual(pkg.contributes.languages[0].extensions, ['.nx']);
  for (const f of [pkg.main, pkg.contributes.languages[0].configuration, pkg.contributes.grammars[0].path]) {
    assert.ok(fs.existsSync(path.join(__dirname, '..', f)), f);
  }
  const grammar = JSON.parse(fs.readFileSync(path.join(__dirname, '..', pkg.contributes.grammars[0].path), 'utf8'));
  assert.equal(grammar.scopeName, pkg.contributes.grammars[0].scopeName);
  const commands = pkg.contributes.commands.map((c) => c.command).sort();
  assert.deepEqual(commands, ['nexalang.explain', 'nexalang.restart']);
});
