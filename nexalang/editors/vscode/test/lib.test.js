// The extension's own logic (lib.js), without VS Code: `npm test`.
'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const os = require('os');
const path = require('path');
const lib = require('../lib');

test('nexac is found in PATH, by full path, never in a relative folder', () => {
  const have = new Set(['/usr/local/bin/nexac', '/opt/nexa/nexac', 'C:\\Tools\\nexac.exe', 'C:\\Program Files\\NexaLang\\nexac.exe']);
  const isRunnable = (p) => have.has(p);
  const find = (o) => lib.findNexac(Object.assign({ isRunnable }, o));

  assert.deepEqual(find({ setting: '', envPath: '/usr/bin:/usr/local/bin', platform: 'linux' }), { path: '/usr/local/bin/nexac' });
  // `.`, empty and relative entries are skipped (they mean the open folder)
  const cwdHave = new Set(['./nexac', 'nexac', 'bin/nexac', '/opt/nexa/nexac']);
  assert.deepEqual(
    lib.findNexac({ setting: '', envPath: '.::bin:/opt/nexa', platform: 'linux', isRunnable: (p) => cwdHave.has(p) }),
    { path: '/opt/nexa/nexac' }
  );
  assert.ok(lib.findNexac({ setting: '', envPath: '.:bin:', platform: 'linux', isRunnable: (p) => cwdHave.has(p) }).error);

  // Windows: `;`, quotes, nexac.exe only (never .bat or .cmd: they'd need a shell)
  assert.deepEqual(find({ setting: '', envPath: 'C:\\Windows;"C:\\Program Files\\NexaLang";C:\\Tools', platform: 'win32' }), {
    path: 'C:\\Program Files\\NexaLang\\nexac.exe',
  });
  assert.deepEqual(find({ setting: '', envPath: '.;nexa;C:\\Tools', platform: 'win32' }), { path: 'C:\\Tools\\nexac.exe' });
  const batOnly = lib.findNexac({ setting: '', envPath: 'C:\\x', platform: 'win32', isRunnable: (p) => p === 'C:\\x\\nexac.bat' });
  assert.match(batOnly.error, /Can't find nexac\.exe/);

  // the setting: a full path to something that runs
  assert.deepEqual(find({ setting: ' /opt/nexa/nexac ', envPath: '', platform: 'linux' }), { path: '/opt/nexa/nexac' });
  assert.match(find({ setting: 'nexac', envPath: '/usr/local/bin', platform: 'linux' }).error, /must be the full path/);
  assert.match(find({ setting: '.\\nexac.exe', envPath: '', platform: 'win32' }).error, /must be the full path/);
  assert.match(find({ setting: '/nowhere/nexac', envPath: '/usr/local/bin', platform: 'linux' }).error, /isn't a program/);
  assert.match(find({ setting: '', envPath: undefined, platform: 'linux' }).error, /colours still work/);

  // the folder that holds it, with or without a last \ or quotes (0.9.1:
  // the first thing tried on a real Windows PC was the folder)
  const folders = new Set(['C:\\Users\\BBF\\NexaLang', 'C:\\Users\\BBF\\NexaLang\\', 'C:\\Empty']);
  const findIn = (setting) =>
    lib.findNexac({
      setting,
      envPath: '',
      platform: 'win32',
      isRunnable: (x) => x === 'C:\\Users\\BBF\\NexaLang\\nexac.exe',
      isFolder: (x) => folders.has(x),
    });
  for (const s of ['C:\\Users\\BBF\\NexaLang', 'C:\\Users\\BBF\\NexaLang\\', ' "C:\\Users\\BBF\\NexaLang" ', '"C:\\Users\\BBF\\NexaLang\\nexac.exe"']) {
    assert.deepEqual(findIn(s), { path: 'C:\\Users\\BBF\\NexaLang\\nexac.exe' }, s);
  }
  assert.match(findIn('C:\\Empty').error, /names the folder C:\\Empty, which has no nexac\.exe in it/);
  assert.match(findIn('"nexac.exe"').error, /must be the full path/);
  assert.deepEqual(find({ setting: 42, envPath: '/usr/local/bin', platform: 'linux' }), { path: '/usr/local/bin/nexac' });
});

test('isRunnableFile: a file that may run, not a folder or a plain file', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'nexalang-vscode-'));
  const prog = path.join(dir, 'nexac');
  const plain = path.join(dir, 'plain');
  fs.writeFileSync(prog, '#!/bin/sh\n', { mode: 0o755 });
  fs.writeFileSync(plain, 'x', { mode: 0o644 });
  if (process.platform !== 'win32') {
    assert.equal(lib.isRunnableFile(prog, 'linux'), true);
    assert.equal(lib.isRunnableFile(plain, 'linux'), false);
  }
  assert.equal(lib.isRunnableFile(plain, 'win32'), true); // Windows has no x bit
  assert.equal(lib.isRunnableFile(dir, 'linux'), false);
  assert.equal(lib.isRunnableFile(path.join(dir, 'missing'), 'linux'), false);
  // a real folder as the setting: nexac inside it is found
  if (process.platform !== 'win32') {
    assert.deepEqual(lib.findNexac({ setting: dir, envPath: '', platform: 'linux' }), { path: prog });
    assert.deepEqual(lib.findNexac({ setting: dir + '/', envPath: '', platform: 'linux' }), { path: prog });
  }
  fs.rmSync(dir, { recursive: true });
});

test('the language server is started as exactly `nexac lsp`, without a shell', () => {
  const c = lib.serverCommand('/usr/local/bin/nexac', '/home/bless');
  assert.deepEqual(c, { command: '/usr/local/bin/nexac', args: ['lsp'], options: { cwd: '/home/bless', shell: false, windowsHide: true } });
  // vscode-languageclient adds `--stdio` when a transport is given; the
  // extension gives none (test: extension.js has no `transport`)
  const src = fs.readFileSync(path.join(__dirname, '..', 'extension.js'), 'utf8');
  assert.doesNotMatch(src, /transport\s*:/);
});

const topic = {
  topic: 'results',
  title: 'Things that can fail (`!T`)',
  summary: 'A `!T` is a value or an error.',
  explanation: 'Handle it with `match`.\n',
  wrong: 'fn main() {\n    let x = files.read("a")\n}\n',
  right: 'fn main() -> !void {\n    let x = files.read("a")?\n    return ok()\n}\n',
};

test('explanations: checked, and shown as Markdown', () => {
  assert.deepEqual(lib.parseTopics(JSON.stringify(topic)), [topic]);
  assert.deepEqual(lib.parseTopics(JSON.stringify([topic, topic])), [topic, topic]);
  assert.throws(() => lib.parseTopics('not json'), /not JSON/);
  assert.throws(() => lib.parseTopics('[]'), /expected fields/);
  assert.throws(() => lib.parseTopics('null'), /expected fields/);
  assert.throws(() => lib.parseTopics(JSON.stringify(Object.assign({}, topic, { wrong: 3 }))), /expected fields/);
  assert.throws(() => lib.parseTopics(JSON.stringify(Object.assign({}, topic, { topic: '../x' }))), /expected fields/);

  const md = lib.topicMarkdown(topic);
  assert.ok(md.startsWith('# Things that can fail (`!T`)\n\nA `!T` is a value or an error.\n'));
  assert.ok(md.includes('## Refused\n\n```nexalang\nfn main() {\n    let x = files.read("a")\n}\n```\n'));
  assert.ok(md.includes('## Accepted\n\n```nexalang\nfn main() -> !void {'));
  assert.ok(md.endsWith('*`nexac explain results` says the same in a terminal.*\n'));
  // no example: no empty section; ``` inside an example can't end the block
  const odd = Object.assign({}, topic, { wrong: '', right: 'x ``` y' });
  const md2 = lib.topicMarkdown(odd);
  assert.ok(!md2.includes('## Refused'));
  assert.ok(md2.includes('````nexalang\nx ``` y\n````'));
});

test("a diagnostic's topic is taken only from nexac's own diagnostics", () => {
  assert.equal(lib.diagnosticTopic({ source: 'nexac', code: 'let-var' }), 'let-var');
  assert.equal(lib.diagnosticTopic({ source: 'nexac', code: { value: 'maybe', target: {} } }), 'maybe');
  assert.equal(lib.diagnosticTopic({ source: 'eslint', code: 'let-var' }), null);
  assert.equal(lib.diagnosticTopic({ source: 'nexac', code: '--help' }), null);
  assert.equal(lib.diagnosticTopic({ source: 'nexac', code: 7 }), null);
  assert.equal(lib.diagnosticTopic(undefined), null);
});
