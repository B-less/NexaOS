// Test-only: colour a NexaLang file with the TextMate grammar
// (editors/vscode/syntaxes/nexalang.tmLanguage.json) through vscode-textmate,
// the engine VS Code uses, and print it in check_lang.c's letters, so both
// grammars can be compared with the same expected output.
//
// usage: node tm_check.js NODE_MODULES GRAMMAR FILE.nx
const path = require('path');
const fs = require('fs');
const [mods, grammarPath, file] = process.argv.slice(2);
const vsctm = require(path.join(mods, 'vscode-textmate'));
const onig = require(path.join(mods, 'vscode-oniguruma'));

// most specific first
const LETTERS = [
  ['invalid', 'x'],
  ['constant.character.escape', 'e'],
  ['punctuation.section.interpolation', 'e'],
  ['comment', 'c'],
  ['support.function.builtin', 'b'],
  ['entity.name.function', 'F'],
  ['entity.name.namespace', 'm'],
  ['entity.name.type', 't'],
  ['storage.type', 't'],
  ['keyword.operator.word', 'o'],
  ['keyword', 'k'],
  ['constant.language.boolean', 'B'],
  ['constant.language.none', 'n'],
  ['constant.numeric.hex', 'h'],
  ['constant.numeric.float', 'f'],
  ['constant.numeric.decimal', 'd'],
  ['string', 's'],
];

function letter(scopes) {
  // the innermost scope that has a letter wins
  for (let i = scopes.length - 1; i >= 0; i--) {
    for (const [prefix, l] of LETTERS) {
      if (scopes[i] === prefix || scopes[i].startsWith(prefix + '.')) return l;
    }
  }
  return '.';
}

(async () => {
  const wasm = fs.readFileSync(path.join(mods, 'vscode-oniguruma', 'release', 'onig.wasm')).buffer;
  await onig.loadWASM(wasm);
  const registry = new vsctm.Registry({
    onigLib: Promise.resolve({
      createOnigScanner: (p) => new onig.OnigScanner(p),
      createOnigString: (s) => new onig.OnigString(s),
    }),
    loadGrammar: async () => vsctm.parseRawGrammar(fs.readFileSync(grammarPath, 'utf8'), grammarPath),
  });
  const grammar = await registry.loadGrammar('source.nexalang');
  const lines = fs.readFileSync(file, 'utf8').split('\n');
  if (lines[lines.length - 1] === '') lines.pop();
  let state = vsctm.INITIAL;
  let out = '';
  for (const line of lines) {
    const r = grammar.tokenizeLine(line, state);
    let marks = '';
    for (const t of r.tokens) {
      // positions are UTF-16 units; the sample is ASCII-only past letters
      marks += letter(t.scopes).repeat(Math.max(0, Math.min(t.endIndex, line.length) - t.startIndex));
    }
    out += line + '\n' + marks + '\n';
    state = r.ruleStack;
  }
  process.stdout.write(out);
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
