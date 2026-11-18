// The parts of the NexaLang extension that don't need VS Code, so they can
// be tested on their own (test/lib.test.js, `npm test`).
'use strict';

const fs = require('fs');
const path = require('path');
const { execFile } = require('child_process');

/** Is `p` a file this user may run? */
function isRunnableFile(p, platform) {
  try {
    if (!fs.statSync(p).isFile()) return false;
    if (platform !== 'win32') fs.accessSync(p, fs.constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

/**
 * Where `nexac` is, as a full path: the `nexalang.path` setting when it's
 * set (it must be a full path), otherwise the first `nexac` (`nexac.exe`
 * on Windows) in a folder of PATH.
 *
 * Folders in PATH that aren't full paths (`.`, an empty entry) are
 * skipped, and the result is always a full path, so a `nexac.exe` inside
 * the folder that's open in VS Code is never started by accident. (Started
 * by name, Windows looks in the current folder first.)
 *
 * Returns { path } or { error } (a sentence to show).
 */
function findNexac({ setting, envPath, platform = process.platform, isRunnable = isRunnableFile }) {
  const p = platform === 'win32' ? path.win32 : path.posix;
  const name = platform === 'win32' ? 'nexac.exe' : 'nexac';
  const wanted = typeof setting === 'string' ? setting.trim() : '';
  if (wanted !== '') {
    if (!p.isAbsolute(wanted)) {
      return { error: `The setting nexalang.path must be the full path to ${name} (it's "${wanted}").` };
    }
    if (!isRunnable(wanted, platform)) {
      return { error: `The setting nexalang.path names ${wanted}, which isn't a program that can be started.` };
    }
    return { path: wanted };
  }
  const sep = platform === 'win32' ? ';' : ':';
  for (let dir of String(envPath || '').split(sep)) {
    dir = dir.trim();
    if (platform === 'win32' && dir.length >= 2 && dir.startsWith('"') && dir.endsWith('"')) {
      dir = dir.slice(1, -1);
    }
    if (dir === '' || !p.isAbsolute(dir)) continue;
    const candidate = p.join(dir, name);
    if (isRunnable(candidate, platform)) return { path: candidate };
  }
  return {
    error:
      `Can't find ${name}, so mistakes, layout and explanations are off (colours still work). ` +
      'Put the folder that holds it in PATH, or set nexalang.path to its full path.',
  };
}

/** The language server's command: a full path and an argument list, never a shell. */
function serverCommand(nexac, homedir) {
  return { command: nexac, args: ['lsp'], options: { cwd: homedir, shell: false, windowsHide: true } };
}

const TOPIC_FIELDS = ['topic', 'title', 'summary', 'explanation', 'wrong', 'right'];

/** A topic from `nexac explain --json`, checked; null if it isn't one. */
function checkTopic(t) {
  if (t === null || typeof t !== 'object' || Array.isArray(t)) return null;
  for (const f of TOPIC_FIELDS) if (typeof t[f] !== 'string') return null;
  if (!/^[a-z][a-z0-9-]{0,40}$/.test(t.topic)) return null;
  return t;
}

/** Topics from the text `nexac explain --json` printed (all, or one). */
function parseTopics(text) {
  let v;
  try {
    v = JSON.parse(text);
  } catch {
    throw new Error("nexac's explanations couldn't be read (not JSON)");
  }
  const list = Array.isArray(v) ? v : [v];
  const topics = list.map(checkTopic);
  if (topics.length === 0 || topics.some((t) => t === null)) {
    throw new Error("nexac's explanations couldn't be read (not the expected fields)");
  }
  return topics;
}

/** A fence that the text inside can't end early. */
function fence(text) {
  let f = '```';
  while (text.includes(f)) f += '`';
  return f;
}

/** An explanation as Markdown, shown beside the code. */
function topicMarkdown(t) {
  const parts = [`# ${t.title}`, '', t.summary, '', t.explanation.trimEnd(), ''];
  if (t.wrong.trim() !== '') {
    const f = fence(t.wrong);
    parts.push('## Refused', '', f + 'nexalang', t.wrong.trimEnd(), f, '');
  }
  if (t.right.trim() !== '') {
    const f = fence(t.right);
    parts.push('## Accepted', '', f + 'nexalang', t.right.trimEnd(), f, '');
  }
  parts.push(`*\`nexac explain ${t.topic}\` says the same in a terminal.*`, '');
  return parts.join('\n');
}

/** The topic of a diagnostic from nexac (its `code`), or null. */
function diagnosticTopic(d) {
  if (!d || d.source !== 'nexac') return null;
  const code = d.code && typeof d.code === 'object' ? d.code.value : d.code;
  return typeof code === 'string' && /^[a-z][a-z0-9-]{0,40}$/.test(code) ? code : null;
}

/** Run `nexac explain --json [topic]`: an argument list, no shell, a time limit. */
function runExplain(nexac, topic, homedir) {
  const args = ['explain', '--json'];
  if (topic) args.push(topic);
  return new Promise((resolve, reject) => {
    execFile(
      nexac,
      args,
      { cwd: homedir, shell: false, windowsHide: true, timeout: 10000, maxBuffer: 4 * 1024 * 1024, encoding: 'utf8' },
      (err, stdout, stderr) => {
        if (err) {
          const said = String(stderr || '').trim().split('\n')[0];
          reject(new Error(said || `nexac explain didn't work: ${err.message}`));
          return;
        }
        try {
          resolve(parseTopics(stdout));
        } catch (e) {
          reject(e);
        }
      }
    );
  });
}

module.exports = {
  findNexac,
  isRunnableFile,
  serverCommand,
  parseTopics,
  topicMarkdown,
  diagnosticTopic,
  runExplain,
};
