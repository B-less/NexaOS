// extension.js with the REAL vscode-languageclient (0.9.2). Only VS Code's
// API is a stand-in: output channels shaped exactly like VS Code's (a log
// channel, `{ log: true }`, is the only kind with .error(), .info(),
// .logLevel and .onDidChangeLogLevel), the rest an automatic stand-in that
// accepts anything. The client must start nexac and reach "running".
// Without this test, 0.9.0 and 0.9.1 failed on the first real VS Code with
// "this.traceOutputChannel.onDidChangeLogLevel is not a function".
'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const Module = require('module');
const fs = require('fs');
const os = require('os');
const path = require('path');

const NEXAC = path.resolve(process.env.NEXAC || path.join(__dirname, '..', '..', '..', 'target', 'release', 'nexac'));

/** Anything: every property and call gives another stand-in. */
function anything(name) {
  const f = function () {};
  return new Proxy(f, {
    get(_t, k) {
      if (k === 'then') return undefined; // not a promise
      if (k === Symbol.toPrimitive || k === 'toString') return () => name;
      if (k === Symbol.iterator) return function* () {};
      return anything(`${name}.${String(k)}`);
    },
    construct: () => anything(`new ${name}`),
    apply: () => anything(`${name}()`),
  });
}

function fakeVscode(log) {
  class EventEmitter {
    constructor() {
      this.listeners = [];
      this.event = (f) => (this.listeners.push(f), { dispose() {} });
    }
    fire(e) {
      this.listeners.forEach((f) => f(e));
    }
    dispose() {}
  }
  class Disposable {
    constructor(f) {
      this.f = f;
    }
    dispose() {
      if (this.f) this.f();
    }
    static from(...ds) {
      return new Disposable(() => ds.forEach((d) => d && d.dispose && d.dispose()));
    }
  }
  const channel = (name, opts) => {
    const c = { name, appendLine: (m) => log.push(m), append: (m) => log.push(m), show() {}, hide() {}, clear() {}, replace() {}, dispose() {} };
    if (opts && opts.log === true) {
      for (const k of ['error', 'warn', 'info', 'debug', 'trace']) c[k] = (m) => log.push(`${k}: ${m}`);
      c.logLevel = 2;
      c.onDidChangeLogLevel = () => ({ dispose() {} });
    }
    return c;
  };
  const any = anything('vscode');
  const over = (part, own) => new Proxy(own, { get: (t, k) => (k in t ? t[k] : any[part][k]) });
  const own = {
    EventEmitter,
    Disposable,
    version: '1.95.0',
    env: { language: 'en', appName: 'Visual Studio Code', machineId: 'm', sessionId: 's' },
    window: over('window', {
      createOutputChannel: channel,
      showErrorMessage: async (m) => log.push(`ERROR SHOWN: ${m}`),
      showWarningMessage: async (m) => log.push(`WARNING SHOWN: ${m}`),
      showInformationMessage: async (m) => log.push(`INFO SHOWN: ${m}`),
    }),
    workspace: over('workspace', {
      getConfiguration: () => ({ get: (_k, d) => d }),
      workspaceFolders: undefined,
      textDocuments: [],
      onDidChangeConfiguration: () => ({ dispose() {} }),
      registerTextDocumentContentProvider: () => ({ dispose() {} }),
    }),
  };
  return new Proxy(own, { get: (t, k) => (k in t ? t[k] : any[k]) });
}

test('the real language client starts nexac and reaches "running"', { timeout: 60000 }, async (t) => {
  const log = [];
  const resolve = Module._resolveFilename;
  Module._resolveFilename = function (req, ...rest) {
    return req === 'vscode' ? 'vscode' : resolve.call(this, req, ...rest);
  };
  require.cache.vscode = { id: 'vscode', filename: 'vscode', loaded: true, exports: fakeVscode(log) };
  const { LanguageClient } = require('vscode-languageclient/node');
  let client = null;
  const start = LanguageClient.prototype.start;
  LanguageClient.prototype.start = function () {
    client = this;
    return start.call(this);
  };
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'nexalang-client-'));
  fs.symlinkSync(NEXAC, path.join(dir, 'nexac'));
  const oldPath = process.env.PATH;
  process.env.PATH = dir;
  t.after(() => {
    process.env.PATH = oldPath;
    Module._resolveFilename = resolve;
    LanguageClient.prototype.start = start;
    fs.rmSync(dir, { recursive: true, force: true });
  });
  const file = require.resolve('../extension.js');
  delete require.cache[file];
  const ext = require(file);
  await ext.activate({ subscriptions: [] });
  assert.ok(client, `no client was started: ${log.join('\n')}`);
  assert.equal(client.state, 2, `the client isn't running (state ${client.state}):\n${log.join('\n')}`);
  assert.equal(client.initializeResult.serverInfo.name, 'nexac');
  assert.ok(client.initializeResult.capabilities.documentFormattingProvider);
  assert.ok(!log.some((l) => /SHOWN|^error:/.test(l)), log.join('\n'));
  await ext.deactivate();
  assert.equal(client.state, 1); // stopped
});
