// The extension's conversations with a real nexac: `nexac explain --json`
// through lib.runExplain, and a language server session through
// vscode-languageserver-protocol (the library VS Code's language client is
// built on), sending what VS Code sends.
//
//   NEXAC=/path/to/nexac npm test           (default: nexalang's release build)
//   NEXAC=./nexac-wine NEXAC_WINDOWS=1 npm test   (a script: exec /usr/bin/wine nexac.exe "$@")
//       the Windows nexac.exe under Wine: paths become Z:\..., URIs
//       file:///z%3A/... as VS Code writes them on Windows
'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const cp = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');
const rpc = require('vscode-languageserver-protocol/node');
const lib = require('../lib');

const NEXAC = path.resolve(process.env.NEXAC || path.join(__dirname, '..', '..', '..', 'target', 'release', 'nexac'));
const WINDOWS = process.env.NEXAC_WINDOWS === '1';
const have = fs.existsSync(NEXAC);

/** The URI VS Code gives a file (on Windows: Wine's Z: drive is /). */
function uriOf(p) {
  const enc = (s) => s.split('/').map(encodeURIComponent).join('/');
  return WINDOWS ? 'file:///z%3A' + enc(p) : 'file://' + enc(p);
}

test('explanations come from nexac explain --json', { skip: !have && `no ${NEXAC}` }, async () => {
  const all = await lib.runExplain(NEXAC, '', os.homedir());
  assert.ok(all.length >= 20, `only ${all.length} topics`);
  assert.ok(all.some((t) => t.topic === 'results'));
  const [one] = await lib.runExplain(NEXAC, 'let-var', os.homedir());
  assert.equal(one.topic, 'let-var');
  assert.ok(lib.topicMarkdown(one).includes('## Refused'));
  await assert.rejects(lib.runExplain(NEXAC, 'no-such-topic', os.homedir()), /no topic called `no-such-topic`/);
});

function startServer(args) {
  const { command, options } = lib.serverCommand(NEXAC, os.homedir());
  const proc = cp.spawn(command, args, Object.assign({ stdio: ['pipe', 'pipe', 'pipe'] }, options));
  const conn = rpc.createProtocolConnection(new rpc.StreamMessageReader(proc.stdout), new rpc.StreamMessageWriter(proc.stdin));
  const published = [];
  let waiting = null;
  conn.onNotification(rpc.PublishDiagnosticsNotification.type, (p) => {
    published.push(p);
    if (waiting) waiting();
  });
  conn.listen();
  /** The next diagnostics published for `uri`. */
  async function diagnostics(uri) {
    for (;;) {
      const i = published.findIndex((p) => p.uri === uri);
      if (i >= 0) return published.splice(i, 1)[0].diagnostics;
      await new Promise((r) => {
        waiting = r;
        setTimeout(r, 200);
      });
    }
  }
  return { proc, conn, diagnostics };
}

// What VS Code 1.9x's client sends, in short.
const INIT = {
  processId: process.pid,
  clientInfo: { name: 'Visual Studio Code', version: '1.95.0' },
  locale: 'en',
  rootUri: null,
  capabilities: {
    general: { positionEncodings: ['utf-16'] },
    textDocument: {
      synchronization: { dynamicRegistration: true, didSave: true, willSave: true, willSaveWaitUntil: true },
      publishDiagnostics: { relatedInformation: true, codeDescriptionSupport: true, dataSupport: true },
      hover: { dynamicRegistration: true, contentFormat: ['markdown', 'plaintext'] },
      formatting: { dynamicRegistration: true },
    },
    workspace: { configuration: true, workspaceFolders: true },
  },
  workspaceFolders: null,
};

for (const args of [['lsp'], ['lsp', '--stdio']]) {
  test(`a VS Code session with \`nexac ${args.join(' ')}\``, { skip: !have && `no ${NEXAC}`, timeout: 60000 }, async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'nexalang vscode é-'));
    fs.writeFileSync(path.join(dir, 'helpers.nx'), 'pub fn twice(n: int) -> int {\n    return n * 2\n}\n');
    fs.writeFileSync(path.join(dir, 'Tools.nx'), 'pub fn one() -> int {\n    return 1\n}\n');
    const main = path.join(dir, 'main.nx');
    const uri = uriOf(main);
    const s = startServer(args);
    try {
      const init = await s.conn.sendRequest(rpc.InitializeRequest.type, INIT);
      assert.equal(init.serverInfo.name, 'nexac');
      assert.ok(init.capabilities.hoverProvider);
      assert.ok(init.capabilities.documentFormattingProvider);
      s.conn.sendNotification(rpc.InitializedNotification.type, {});

      // a file from Windows (\r\n), using a module next to it, with a mistake
      const bad = 'use helpers\r\n\r\nfn main() {\r\n    let s = "😀" + cuont\r\n    print(helpers.twice(2))\r\n}\r\n';
      s.conn.sendNotification(rpc.DidOpenTextDocumentNotification.type, {
        textDocument: { uri, languageId: 'nexalang', version: 1, text: bad },
      });
      const d = await s.diagnostics(uri);
      assert.equal(d.length, 1, JSON.stringify(d));
      assert.match(d[0].message, /cuont/, JSON.stringify(d));
      assert.equal(d[0].source, 'nexac');
      assert.equal(d[0].code, 'unknown');
      assert.match(d[0].message, /cuont/);
      // `cuont` starts after "    let s = " (12) + the emoji (2 UTF-16 units) + ` + ` (3)
      assert.deepEqual(d[0].range, { start: { line: 3, character: 19 }, end: { line: 3, character: 24 } });
      assert.equal(lib.diagnosticTopic(d[0]), 'unknown');

      const hover = await s.conn.sendRequest(rpc.HoverRequest.type, { textDocument: { uri }, position: { line: 3, character: 20 } });
      assert.ok(hover && hover.contents, 'no hover on the mistake');
      assert.match(JSON.stringify(hover.contents), /cuont/);
      const nothing = await s.conn.sendRequest(rpc.HoverRequest.type, { textDocument: { uri }, position: { line: 0, character: 1 } });
      assert.equal(nothing, null);

      // fixed, but laid out badly: no mistakes; formatting keeps \r\n
      const messy = 'use helpers\r\nfn main(){\r\nprint( helpers.twice(2) )\r\n}';
      s.conn.sendNotification(rpc.DidChangeTextDocumentNotification.type, {
        textDocument: { uri, version: 2 },
        contentChanges: [{ text: messy }],
      });
      assert.deepEqual(await s.diagnostics(uri), []);
      const edits = await s.conn.sendRequest(rpc.DocumentFormattingRequest.type, {
        textDocument: { uri },
        options: { tabSize: 4, insertSpaces: true },
      });
      assert.equal(edits.length, 1);
      assert.equal(edits[0].newText, 'use helpers\r\nfn main() {\r\n    print(helpers.twice(2))\r\n}\r\n');

      // the module must be spelled as the file is, also where the disk doesn't care
      s.conn.sendNotification(rpc.DidChangeTextDocumentNotification.type, {
        textDocument: { uri, version: 3 },
        contentChanges: [{ text: 'use tools\nfn main() { }\n' }],
      });
      const caps = await s.diagnostics(uri);
      assert.equal(caps.length, 1);
      assert.match(caps[0].message, /called Tools\.nx, and capitals count/);

      // saved (the text is what VS Code had), then closed: mistakes cleared
      s.conn.sendNotification(rpc.DidSaveTextDocumentNotification.type, { textDocument: { uri } });
      assert.equal((await s.diagnostics(uri)).length, 1);
      s.conn.sendNotification(rpc.DidCloseTextDocumentNotification.type, { textDocument: { uri } });
      assert.deepEqual(await s.diagnostics(uri), []);

      // an unsaved, never-named file
      const untitled = 'untitled:Untitled-1';
      s.conn.sendNotification(rpc.DidOpenTextDocumentNotification.type, {
        textDocument: { uri: untitled, languageId: 'nexalang', version: 1, text: 'fn main() {\n    print(1)\n}\n' },
      });
      assert.deepEqual(await s.diagnostics(untitled), []);

      await s.conn.sendRequest(rpc.ShutdownRequest.type);
      const exited = new Promise((r) => s.proc.on('exit', (code) => r(code)));
      s.conn.sendNotification(rpc.ExitNotification.type);
      assert.equal(await exited, 0);
    } finally {
      s.conn.dispose();
      s.proc.kill();
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
}
