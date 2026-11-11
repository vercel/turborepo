#!/usr/bin/env node
// Runtime oracle for the bound on reads whose path cannot be evaluated
// (crates/query/src/names.rs, crates/lang-js/src/names.rs).
//
// The claim under test: a read with a computed path can only read a repository file that a
// string or a file location of the code loaded with it names (or that a reference, a
// listing or an open record covers). So for every path a program really reads,
// `affected <entry> --changed <path>` must select the entry.
//
// The programs are generated from a grammar of the ways a path reaches an opaque read:
//
//   base       where the directory comes from: `__dirname`, the working directory, an
//              ancestor computed in another module, the environment, nothing (relative);
//              the directory is the program's own package, or (the last group of BASES)
//              the package of the file with the read, another package, the root package
//   relative   how the part below it is written: a literal, separate segments, a template,
//              a listing, a data file (JSON, YAML), a constant of another module, a word
//              inside a command line, fragments, bare words
//   transport  how the path reaches the read: a parameter, a field set by a constructor, an
//              intermediate module
//
// Every combination is one small program in its own directory of one project. Each runs
// under Node with preload.cjs, which records the paths it passes to the file-reading APIs.
// A recorded path that does not select the entry is a FINDING, unless the combination is a
// documented hole of the bound (`expectedHole`): then it is counted as such, and a
// documented hole that turns out covered is reported too (the list is kept honest).
//
// The bound also makes one assumption: a read with a computed path reads only files of the
// package it runs for (the entry's package, or the package of the file with the read),
// unless the code says where the path goes. Programs that break it on purpose (`assumed`
// bases: another package reached through text from outside) are ASSUMPTION CASES. They are
// counted apart and prove nothing; they pin the boundary: not selected through a name by
// default, selected with TURBO_TRACE_DEBUG_NAMES_PACKAGE=0 (unless a documented hole) and with
// --strict. Anything else there is a FINDING.
//
// TASK CASES. The same programs as build scripts: `tasks/b<i>/build.js` is run by the
// `build` script of its package (in the package directory, as a package manager runs it)
// and writes `dist/index.js`; the entry is in another package, `tasks/c<i>/entry.js`, and
// only loads that output. The script runs when the task runs, so what names its reads is
// the task: its working directory and its package, not those of the consumer. Checked for
// every path the script really read: a change to it selects the consumer (a FINDING
// otherwise, unless a documented hole), and a change to the same path in the consumer's
// package does not select it through a name (a FINDING too: the consumer's directory and
// package are not the script's). TURBO_TRACE_DEBUG_NAMES_TASK_RUNNER=0 is the answer from before,
// counted next to it.
//
// ROOT CASES. Programs that take the repository root from their own file (`ROOT`: parent
// steps only, as a root finder writes it; a path to a named directory would be an asset
// reference of its own) and do
// something with that directory that is not a read of this process: the base of
// `path.relative`, a search or `.length`, a message, the working directory of a child
// process, an argument of a function of the file. A location that leaves the file names
// everything below it; these uses name nothing, or exactly what the code says. Checked:
// every path the program really read (and every file its child's command line names)
// selects the entry, and a file below `ROOT` that it did not read does not select it
// through a name. The same programs are traced a second time with the consumers switched
// off (`TURBO_TRACE_DEBUG_ANCHORS_OFF`): there the unread file must select through a name, which
// shows that the case depends on the rule (a FINDING otherwise: the case proves nothing).
// Cases marked `everything` hand the directory on to code that can read anything below
// it: only the first check applies.
//
//   node oracle.mjs --turbotrace <bin> [--only TEXT] [--keep] [-v] [--json FILE]
//
// Exit status 1 when there are findings. The trace's development tooling builds the binary and
// runs this script.
import { execFileSync, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const opt = (name, dflt) => {
  const i = args.indexOf(name);
  return i >= 0 && i + 1 < args.length ? args[i + 1] : dflt;
};
const BIN = path.resolve(opt('--turbotrace', 'target/debug/turbotrace'));
const ONLY = opt('--only', '');
const VERBOSE = flag('-v');

// ---- the grammar ----

// base: `expr` evaluates to the case directory (or null: the path stays relative to the
// working directory, which is the case directory). `named`: something of the program names
// everything below the base (a file location or the working directory), so even a relative
// part that no string names is covered.
const BASES = {
  dirname: { expr: '__dirname', named: true },
  dirnameOfFile: { expr: 'path.dirname(__filename)', named: true },
  cwd: { expr: 'process.cwd()', named: true },
  ancestor: { expr: "require('./util/base').ROOT", named: true },
  // The parent step is in another expression than the location: it still lifts it.
  ancestorInTwoSteps: { expr: "require('./util/base2').ROOT", named: true },
  env: { expr: 'process.env.NAMES_ORACLE_ROOT', named: false },
  relative: { expr: 'null', named: false },
  // Where a location goes (anchored names): through an array and a callback, a loop, a
  // filter, a conditional, a URL.
  dirnameThroughMap: { expr: '[__dirname].map((d) => d)[0]', named: true },
  dirnameInLoop: { expr: '(() => { let b = null; for (const d of [process.cwd(), __dirname]) { b = d; } return b; })()', named: true },
  dirnameFound: { expr: '[__dirname].find((d) => d.length > 0)', named: true },
  dirnameOrCwd: { expr: "(process.env.NAMES_ORACLE_NONE ? process.cwd() : path.resolve(__dirname, 'fixtures', '..', '.'))", named: true },
  dirnameOfUrl: { expr: "require('url').fileURLToPath(new URL('.', require('url').pathToFileURL(__filename)))", named: true },
  cwdResolved: { expr: "path.resolve(process.cwd(), 'fixtures', '..')", named: true },
  // A location with a tail from a function of another module that is no environment
  // helper: everything below the location stays named.
  dirnameCallTail: { expr: "path.join(__dirname, require('./build').build('.'))", named: true },
  // A location with a tail from the environment: the environment registry's. What the
  // variable holds is text from outside, so only strings name what is read.
  dirnameEnvTail: { expr: 'path.join(__dirname, process.env.NAMES_ORACLE_DOT)', named: false },
  // ---- other packages ----
  // The package of the file with the read (lib/reader.js), reached through text from
  // outside: a string of the program names its files (the package assumption covers the
  // entry's package and the package of the read).
  envPackageOfTheRead: { expr: 'process.env.NAMES_ORACLE_LIB', named: false, dir: 'lib' },
  // Another package, located by the code: the path says where it goes, whatever package
  // that is (evidence beats the assumption).
  cwdToOtherPackage: { expr: "path.join(process.cwd(), '..', '..', 'other')", named: true, dir: 'other' },
  // A literal path from the file's location: an anchored entry names where it leads.
  dirnameToOtherPackage: { expr: "path.resolve(__dirname, '..', '..', 'other')", named: true, dir: 'other' },
  dirnameToRootPackage: { expr: "path.join(__dirname, '../../shared')", named: true, dir: 'shared' },
  // The same in another module: the value arrives as an import, so only the entry of the
  // module that wrote the path says where it goes.
  exportedDirnameToOtherPackage: { expr: "require('./util/other').DIR", named: true, dir: 'other' },
  // A relative string that climbs out of where it starts says that it leaves: its shape is
  // not held to a package.
  climbingStringToOtherPackage: { expr: "'../../other'", named: true, dir: 'other' },
  // ASSUMPTION CASES. Another package, or files of the root package, reached through text
  // from outside that nothing in the code locates. The package assumption says such a read
  // does not happen, so by default a change there does not select the entry. These cases
  // prove nothing: they pin where the boundary is. Each must be selected without the
  // assumption (TURBO_TRACE_DEBUG_NAMES_PACKAGE=0) and with --strict.
  envOtherPackage: { expr: 'process.env.NAMES_ORACLE_OTHER', named: false, dir: 'other', assumed: true },
  envRootPackage: { expr: 'process.env.NAMES_ORACLE_SHARED', named: false, dir: 'shared', assumed: true },
};

// relative: `expr` evaluates to an array of paths below the base. `strings`: the part is
// named by strings of the program whatever the base is.
const RELS = {
  literal: { expr: "['fixtures/a.json']", strings: true },
  segmentsInPathCall: { expr: "[path.join('fixtures', 'sub', 'raw')]", strings: true },
  // A path function under another name counts through its binding.
  segmentsInImportedJoin: { expr: "(() => { const { join: j } = require('node:path'); return [j('fixtures', 'sub', 'raw')]; })()", strings: true },
  template: { expr: "['a', 'b'].map((n) => `fixtures/${n}.json`)", strings: true },
  // What follows a hole constrains the name: a hole can hold separators, be empty, or be
  // part of a segment, and a parent step in the file cuts the path back.
  templateHoleWithSeparator: { expr: "['sub/c'].map((n) => `fixtures/${n}.json`)", strings: true },
  templateEmptyHole: { expr: "[''].map((n) => `fixtures/${n}/a.json`)", strings: true },
  templateHoleInSegment: { expr: "['b'].map((n) => `fixtures/su${n}/c.json`)", strings: true },
  templateThenDirname: { expr: "['sub'].map((n) => path.dirname(`fixtures/${n}/c.json`) + '/raw')", strings: true },
  templateWithoutExtension: { expr: "['sub'].map((n) => `fixtures/${n}/c`).map((p) => p + '.json')", strings: true },
  jsonTable: { expr: "require('./data/table.json').files", strings: true },
  yamlList: {
    expr: "fs.readFileSync(path.join(__dirname, 'data/list.yaml'), 'utf8').split('\\n').filter(Boolean).map((l) => l.replace('- ', ''))",
    strings: true,
  },
  constantOfModule: { expr: "[require('./consts').FIX]", strings: true },
  wordInCommandLine: { expr: "['--file=fixtures/b.json'.split('=')[1]]", strings: true },
  fileNameOnly: { expr: "[['fixtures', 'b.json'].join('/')]", strings: true },
  // Holes of the bound unless the base is named: no string of the program has a separator,
  // an extension or a path-function position.
  fragments: { expr: "['fix' + 'tures' + '/' + 'sub' + '/' + 'raw']", strings: false },
  bareWords: { expr: "[['fixtures', 'sub', 'raw'].join('/')]", strings: false },
  bareWordsThroughHelper: { expr: "[require('./build').build('fixtures', 'sub', 'raw')]", strings: false },
  // A helper that has the name of a path function is not one: its words are bare words.
  bareWordsThroughOwnJoin: { expr: "[require('./build').join('fixtures', 'sub', 'raw')]", strings: false },
  // Text from outside that the program only compares with a literal: the literal is not
  // where the path comes from.
  // `env`: the part is text of the environment. A path that is a location followed by it is
  // the environment registry's (an environment read, reported and not counted), whatever
  // the base is.
  outsideTextCompared: { expr: "[process.env.NAMES_ORACLE_REL].filter((p) => p === 'fixtures/b.json' || p.startsWith('fixtures/b'))", strings: false, env: true },
};

// transport: statements that read the path `B`/`r`.
const TRANSPORTS = {
  parameter: 'reader.readText(B === null ? r : path.join(B, r));',
  constructorField: "new reader.Store(B === null ? '.' : B).get(r);",
  otherModule: "require('./via').load(B === null ? r : path.join(B, r));",
  // Read where the path is built: the read's own reference has to cover it (a location
  // that only goes into a read of its file names nothing by itself).
  directRead: "fs.readFileSync(B === null ? r : path.join(B, r), 'utf8');",
};

// Listings: the directory comes from the base; its entries are named by nothing.
const LISTINGS = {
  listNamedDirectory: { stmt: "reader.readAll(B === null ? 'fixtures' : path.join(B, 'fixtures'));", strings: true },
  listBase: { stmt: "reader.readAll(B === null ? '.' : B);", strings: false },
  // A directory handed to a package known to read only fixed file names in it (the stub
  // below reads what dotenv-flow documents).
  envFilesOfBase: { stmt: "require('dotenv-flow').config({ path: B === null ? '.' : B, default_node_env: 'test' });", strings: false },
};

// A documented hole: neither a string nor a location or working directory of the program
// names what is read.
const expectedHole = (c) => (!c.base.named && !c.rel.strings) || c.rel.env === true;

function programs() {
  const out = [];
  for (const [bn, base] of Object.entries(BASES)) {
    for (const [rn, rel] of Object.entries(RELS)) {
      for (const [tn, stmt] of Object.entries(TRANSPORTS)) {
        out.push({ name: `${bn}.${rn}.${tn}`, base, rel, body: `for (const r of ${rel.expr}) { ${stmt} }` });
      }
    }
    for (const [ln, l] of Object.entries(LISTINGS)) {
      out.push({ name: `${bn}.${ln}`, base, rel: { strings: l.strings }, body: l.stmt });
    }
  }
  return out.filter((c) => !ONLY || c.name.includes(ONLY));
}

// The task cases: the bases that depend on who runs the program, and one that does not.
const TASK_BASES = ['cwd', 'cwdResolved', 'relative', 'env', 'dirname'];
const TASK_RELS = ['literal', 'template', 'constantOfModule', 'fragments', 'bareWords'];
function taskPrograms() {
  const out = [];
  for (const bn of TASK_BASES) {
    const base = BASES[bn];
    for (const rn of TASK_RELS) {
      const rel = RELS[rn];
      for (const [tn, stmt] of Object.entries(TRANSPORTS)) {
        out.push({ name: `task.${bn}.${rn}.${tn}`, base, rel, body: `for (const r of ${rel.expr}) { ${stmt} }` });
      }
    }
    for (const [ln, l] of Object.entries(LISTINGS)) {
      if (ln === 'envFilesOfBase') continue;
      out.push({ name: `task.${bn}.${ln}`, base, rel: { strings: l.strings }, body: l.stmt });
    }
  }
  return out.filter((c) => !ONLY || c.name.includes(ONLY));
}

// The root cases: statements with `ROOT` (the root of the project, taken from the file's
// location with parent steps) and `own` (a file of the program's package, read through
// the opaque reader). `expect`: files the child process's command line names (its reads
// are not recorded). `everything`: the use can read anything below `ROOT`.
const ROOT_CASES = {
  relativeBase: { body: 'const rel = path.relative(ROOT, own); if (rel.length === 0) throw new Error("same");' },
  // The result of `relative` on a new base: the path is the other argument's.
  relativeRebased: {
    body: "const target = path.join(ROOT, 'other/fixtures/a.json'); reader.readText(path.join(process.cwd(), '..', '..', path.relative(ROOT, target)));",
    everything: true,
  },
  // As the target of `relative` the directory is what the result leads to.
  relativeTarget: {
    body: "reader.readText(path.join(__dirname, path.relative(__dirname, ROOT), ['other', 'fixtures', 'b.json'].join('/')));",
    everything: true,
  },
  searches: { body: "if (!own.startsWith(ROOT) || ROOT.includes('\\0') || own.indexOf(ROOT) !== 0 || ROOT.length === 0) throw new Error('outside');" },
  messages: { body: "console.error('root: ' + ROOT, ROOT); try { throw new Error(`not below ${ROOT}`); } catch (e) { if (!(e instanceof Error)) throw e; }" },
  childKnownCommand: {
    body: "require('child_process').execFileSync('node', ['other/tool.js', '--quiet'], { cwd: ROOT, stdio: 'ignore' });",
    expect: ['other/tool.js'],
  },
  childKnownShellCommand: {
    body: "require('child_process').execSync('node ./other/tool.js --quiet', { stdio: 'ignore', cwd: ROOT });",
    expect: ['other/tool.js'],
  },
  childWithoutPath: { body: "require('child_process').spawnSync('node', ['-e', '0'], { cwd: ROOT });" },
  childUnknownCommand: {
    body: "require('child_process').execFileSync('node', [process.env.NAMES_ORACLE_TOOL], { cwd: ROOT, stdio: 'ignore' });",
    expect: ['other/tool.js'],
    everything: true,
  },
  localFunctionTests: { body: 'function inside(root, p) { return p.startsWith(root); } if (!inside(ROOT, own)) throw new Error("outside");' },
  localFunctionReads: { body: "const read = (base, file) => fs.readFileSync(path.join(base, file), 'utf8'); read(ROOT, 'other/fixtures/a.json');" },
  localFunctionReadsUnknown: {
    body: "function load(base, name) { return reader.readText(path.join(base, name)); } load(ROOT, ['other', 'fixtures', 'b.json'].join('/'));",
    everything: true,
  },
  // Joined with literal segments before it leaves: that subtree only.
  localFunctionHandsOn: { body: "function pass(dir) { return reader.readAll(path.join(dir, 'other', 'fixtures')); } pass(ROOT);" },
  defaultOfParameter: {
    body: "function scan(dir = ROOT) { return reader.readText(path.join(dir, ['other', 'fixtures', 'a.json'].join('/'))); } scan();",
    everything: true,
  },
  defaultOfParameterTested: { body: 'function depth(dir = ROOT) { return dir.length; } if (depth() === 0) throw new Error("empty");' },
  // What an assertion compares (a stand-in for the test runner's global).
  assertion: {
    body:
      "globalThis.expect = (v) => ({ toBe: (w) => { if (v !== w) throw new Error('differs'); } });\n" +
      "globalThis.expect.objectContaining = (o) => o;\n" +
      'expect(path.dirname(path.dirname(__dirname))).toBe(ROOT); expect(ROOT).toBe(path.dirname(path.dirname(__dirname)));\n' +
      'expect.objectContaining({ cwd: ROOT });',
  },
  // A path that is only written.
  writes: {
    body:
      "function emit(base, name) { fs.mkdirSync(path.join(base, name), { recursive: true }); fs.writeFileSync(path.join(base, name, 'x.txt'), 'x'); fs.rmSync(path.join(base, name), { recursive: true, force: true }); }\n" +
      "emit(ROOT, ['written', 'by', path.basename(__dirname)].join('-'));",
  },
  // Written, then read back through the same value: the read names it.
  writtenThenRead: {
    body:
      "const out = path.join(ROOT, 'other', 'fixtures');\n" +
      "fs.writeFileSync(path.join(out, 'made-by-' + path.basename(__dirname) + '.json'), '{}');\n" +
      "reader.readText(path.join(out, ['made-by-', path.basename(__dirname), '.json'].join('')));\n" +
      "fs.rmSync(path.join(out, 'made-by-' + path.basename(__dirname) + '.json'));",
    everything: true,
  },
};
function rootPrograms() {
  return Object.entries(ROOT_CASES)
    .map(([name, c]) => ({ name: `root.${name}`, ...c }))
    .filter((c) => !ONLY || c.name.includes(ONLY));
}
// The consumers the root cases are about, switched off for the second trace.
const ROOT_RULES_OFF = 'relative,tests,local,message,child,writes';

// ---- the project ----

const work = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'names-oracle-')));
const proj = path.join(work, 'proj');
const write = (rel, text) => {
  const p = path.join(proj, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, text);
};
write('package.json', '{"name":"proj","version":"1.0.0","private":true}');
// Every case is a package of the workspace: its directory is where its entry runs.
write('pnpm-workspace.yaml', "packages:\n  - 'cases/*'\n  - 'tasks/*'\n  - 'roots/*'\n  - 'lib'\n  - 'other'\n");
// The `build` script of a package writes its `dist`.
write('turbo.json', '{"tasks":{"build":{"outputs":["dist/**"]}}}');
write('lib/package.json', '{"name":"lib","version":"1.0.0"}');
// Another package, and a directory of the root package (no package of its own).
write('other/package.json', '{"name":"other","version":"1.0.0"}');
const FIXTURE_FILES = ['fixtures/a.json', 'fixtures/b.json', 'fixtures/sub/c.json', 'fixtures/sub/raw', 'notes.txt'];
for (const dir of ['lib', 'other', 'shared']) {
  for (const f of FIXTURE_FILES) write(`${dir}/${f}`, '{}');
  for (const f of ['.env', '.env.test']) write(`${dir}/${f}`, 'A=1\n');
}
// dotenv-flow, as documented: `.env`, `.env.defaults`, `.env.local`, `.env.<node_env>` and
// `.env.<node_env>.local` in the directory `path`.
write('node_modules/dotenv-flow/package.json', '{"name":"dotenv-flow","version":"4.1.0","main":"index.js"}');
write(
  'node_modules/dotenv-flow/index.js',
  `const fs = require('fs');
const path = require('path');
exports.config = function (options = {}) {
  const dir = options.path || process.cwd();
  const env = process.env.NODE_ENV || options.default_node_env;
  const names = ['.env.defaults', '.env', '.env.local'];
  if (env) names.push('.env.' + env, '.env.' + env + '.local');
  for (const n of names) {
    const file = path.join(dir, n);
    if (fs.existsSync(file)) fs.readFileSync(file, 'utf8');
  }
  return {};
};
`,
);
write(
  'lib/reader.js',
  `const fs = require('fs');
const path = require('path');
exports.readText = function (p) {
  return fs.readFileSync(p, 'utf8');
};
exports.readAll = function (dir) {
  return fs.readdirSync(dir).filter((n) => n.includes('.') || n === 'raw').map((n) => {
    const p = path.join(dir, n);
    return fs.statSync(p).isFile() ? fs.readFileSync(p, 'utf8') : '';
  });
};
exports.Store = class Store {
  constructor(root) {
    this.root = root;
  }
  get(name) {
    return fs.readFileSync(path.join(this.root, name), 'utf8');
  }
};
`,
);
const cases = programs();
cases.forEach((c, i) => {
  c.dir = `cases/c${i}`;
  write(`${c.dir}/package.json`, `{"name":"c${i}","version":"1.0.0"}`);
  write(
    `${c.dir}/entry.js`,
    `const fs = require('fs');
const path = require('path');
const reader = require('../../lib/reader');
const B = ${c.base.expr};
${c.body}
`,
  );
  write(`${c.dir}/util/base.js`, "exports.ROOT = require('path').resolve(__dirname, '..');\n");
  write(`${c.dir}/util/base2.js`, "const path = require('path');\nconst here = path.resolve(__dirname);\nexports.ROOT = path.join(here, '..');\n");
  write(`${c.dir}/util/other.js`, "exports.DIR = require('path').join(__dirname, '../../../other');\n");
  write(`${c.dir}/consts.js`, "exports.FIX = 'fixtures/b.json';\n");
  write(`${c.dir}/build.js`, "exports.build = (...parts) => parts.join('/');\nexports.join = exports.build;\n");
  write(`${c.dir}/via.js`, "const reader = require('../../lib/reader');\nexports.load = (p) => reader.readText(p);\n");
  write(`${c.dir}/data/table.json`, '{"files": ["fixtures/sub/c.json"]}');
  write(`${c.dir}/data/list.yaml`, '- fixtures/a.json\n- fixtures/sub/c.json\n');
  for (const f of FIXTURE_FILES) write(`${c.dir}/${f}`, '{}');
  for (const f of ['.env', '.env.test']) write(`${c.dir}/${f}`, 'A=1\n');
});
const taskCases = taskPrograms();
taskCases.forEach((c, i) => {
  c.dir = `tasks/b${i}`;
  c.consumer = `tasks/c${i}`;
  write(`${c.dir}/package.json`, `{"name":"b${i}","version":"1.0.0","main":"dist/index.js","scripts":{"build":"node build.js"}}`);
  write(
    `${c.dir}/build.js`,
    `const fs = require('fs');
const path = require('path');
const reader = require('../../lib/reader');
const B = ${c.base.expr};
${c.body}
fs.mkdirSync(path.join(__dirname, 'dist'), { recursive: true });
fs.writeFileSync(path.join(__dirname, 'dist', 'index.js'), 'module.exports = 1;\\n');
`,
  );
  write(`${c.dir}/consts.js`, "exports.FIX = 'fixtures/b.json';\n");
  write(`${c.dir}/via.js`, "const reader = require('../../lib/reader');\nexports.load = (p) => reader.readText(p);\n");
  write(`${c.consumer}/package.json`, `{"name":"c${i}","version":"1.0.0","dependencies":{"b${i}":"workspace:*"}}`);
  write(`${c.consumer}/entry.js`, `module.exports = require('../b${i}/dist/index.js');\n`);
  // The same files in both packages: the consumer's are never read.
  for (const dir of [c.dir, c.consumer]) {
    for (const f of FIXTURE_FILES) write(`${dir}/${f}`, '{}');
  }
});
const rootCases = rootPrograms();
// A program the child processes of the root cases run: it reads nothing.
write('other/tool.js', 'process.exitCode = 0;\n');
rootCases.forEach((c, i) => {
  c.dir = `roots/r${i}`;
  write(`${c.dir}/package.json`, `{"name":"r${i}","version":"1.0.0"}`);
  write(
    `${c.dir}/entry.js`,
    `const fs = require('fs');
const path = require('path');
const reader = require('../../lib/reader');
const ROOT = path.resolve(__dirname, '..', '..');
const own = path.join(__dirname, 'fixtures/a.json');
reader.readText(own);
${c.body}
`,
  );
  for (const f of FIXTURE_FILES) write(`${c.dir}/${f}`, '{}');
});
execFileSync(BIN, ['graph', 'build', '--root', proj], { cwd: proj, stdio: ['ignore', 'ignore', 'inherit'] });
// The same tree traced with the consumers of the root cases switched off.
const graphOff = path.join(work, 'graph-rules-off');
if (rootCases.length) {
  execFileSync(BIN, ['graph', 'build', '--root', proj, '--graph', graphOff], {
    cwd: proj,
    stdio: ['ignore', 'ignore', 'inherit'],
    env: { ...process.env, TURBO_TRACE_DEBUG_ANCHORS_OFF: ROOT_RULES_OFF },
  });
}

// ---- run and check ----

function run(c, script = 'entry.js') {
  const out = path.join(work, 'seen.json');
  fs.rmSync(out, { force: true });
  const dir = path.join(proj, c.dir);
  const env = {
    PATH: process.env.PATH,
    HOME: process.env.HOME,
    NAMES_ORACLE_PROJ: proj,
    NAMES_ORACLE_OUT: out,
    NAMES_ORACLE_ROOT: dir,
    NAMES_ORACLE_REL: 'fixtures/b.json',
    NAMES_ORACLE_DOT: '.',
    NAMES_ORACLE_LIB: path.join(proj, 'lib'),
    NAMES_ORACLE_OTHER: path.join(proj, 'other'),
    NAMES_ORACLE_SHARED: path.join(proj, 'shared'),
    NAMES_ORACLE_TOOL: 'other/tool.js',
  };
  const r = spawnSync(process.execPath, ['--require', path.join(HERE, 'preload.cjs'), script], { cwd: dir, env, encoding: 'utf8', timeout: 20000 });
  let seen = [];
  try {
    seen = JSON.parse(fs.readFileSync(out, 'utf8'));
  } catch {}
  return { seen, crashed: r.status !== 0, stderr: r.stderr || '' };
}

// How `affected` selects the entry when `rel` changes: the cause type, 'open' for open
// uncertainty, or null when it does not. `how.args`: more flags; `how.env`: environment.
function selectedBy(entry, rel, how = {}) {
  const argv = ['affected', entry, '--changed', rel, '--root', proj, '--no-refresh', '--format', 'json', ...(how.args ?? [])];
  const out = execFileSync(BIN, argv, { cwd: proj, encoding: 'utf8', maxBuffer: 16 << 20, env: { ...process.env, ...(how.env ?? {}) } });
  const res = JSON.parse(out).results.find((e) => e.entry === entry);
  if (!res) return null;
  return res.cause ? res.cause.type : 'open';
}

const total = { programs: cases.length, reads: 0, named: 0, reference: 0, open: 0, holes: 0, findings: 0, crashed: 0 };
// The assumption cases: reads in another package through a path nothing locates.
const assumption = { programs: cases.filter((c) => c.base.assumed).length, reads: 0, notSelected: 0, selectedAnyway: [], holes: 0 };
const NO_ASSUMPTION = { env: { TURBO_TRACE_DEBUG_NAMES_PACKAGE: '0' } };
const STRICT = { args: ['--strict'] };
const findings = [];
const coveredHoles = [];
const byCause = {};
for (const c of cases) {
  const r = run(c);
  if (r.crashed) {
    total.crashed++;
    if (VERBOSE) console.error(`[${c.name}] crashed: ${r.stderr.trim().split('\n').slice(-3).join(' | ')}`);
  }
  const entry = `${c.dir}/entry.js`;
  const done = new Set();
  let missed = 0;
  for (const s of r.seen) {
    if (!s.path.startsWith(proj + path.sep)) continue;
    // A listing depends on what is in the directory: a file added to it.
    const rel = path.relative(proj, s.kind === 'dir' ? path.join(s.path, 'added-later.bin') : s.path);
    if (done.has(rel)) continue;
    done.add(rel);
    if (c.base.assumed && (rel === c.base.dir || rel.startsWith(c.base.dir + '/'))) {
      // An assumption case: the boundary is pinned, nothing is proved.
      assumption.reads++;
      const read = path.relative(path.join(proj, c.dir), s.path);
      const problem = (what) => {
        total.findings++;
        findings.push({ program: c.name, read, kind: s.kind, what });
      };
      const by = selectedBy(entry, rel);
      if (!by) assumption.notSelected++;
      else assumption.selectedAnyway.push(`${c.name}: ${read} (${by})`);
      if (by === 'named' && !expectedHole(c)) problem('an assumption case is selected through a name by default: the boundary of the package assumption moved');
      if (!selectedBy(entry, rel, STRICT)) problem('not selected with --strict');
      if (!selectedBy(entry, rel, NO_ASSUMPTION)) {
        if (expectedHole(c)) assumption.holes++;
        else problem('not selected without the package assumption (TURBO_TRACE_DEBUG_NAMES_PACKAGE=0)');
      }
      continue;
    }
    total.reads++;
    const by = selectedBy(entry, rel);
    byCause[by ?? 'not selected'] = (byCause[by ?? 'not selected'] ?? 0) + 1;
    if (by === 'named') total.named++;
    else if (by === 'open') total.open++;
    else if (by) total.reference++;
    else {
      missed++;
      if (expectedHole(c)) total.holes++;
      else {
        total.findings++;
        findings.push({ program: c.name, read: path.relative(path.join(proj, c.dir), s.path), kind: s.kind });
      }
    }
  }
  if (expectedHole(c) && missed === 0 && r.seen.length > 0) coveredHoles.push(c.name);
}

// ---- task cases ----
// `reads`: paths the scripts really read; `selected`: the consumer is selected for a
// change to one; `needRunner`: of those, not selected when the script is named as if the
// consumer ran it; `twins`: the same paths in the consumer's package, `twinsBefore` of
// them selected through a name without the runner, `twinsByReference` through a reference
// (the consumer's manifest, a pattern of the read).
const tasks = { programs: taskCases.length, reads: 0, selected: 0, holes: 0, needRunner: 0, twins: 0, twinsByReference: 0, twinsBefore: 0 };
const NO_RUNNER = { env: { TURBO_TRACE_DEBUG_NAMES_TASK_RUNNER: '0' } };
for (const c of taskCases) {
  const r = run(c, 'build.js');
  fs.rmSync(path.join(proj, c.dir, 'dist'), { recursive: true, force: true });
  if (r.crashed) {
    total.crashed++;
    if (VERBOSE) console.error(`[${c.name}] crashed: ${r.stderr.trim().split('\n').slice(-3).join(' | ')}`);
  }
  const entry = `${c.consumer}/entry.js`;
  const done = new Set();
  let missed = 0;
  for (const s of r.seen) {
    if (!s.path.startsWith(proj + path.sep)) continue;
    const rel = path.relative(proj, s.kind === 'dir' ? path.join(s.path, 'added-later.bin') : s.path);
    if (done.has(rel) || rel.startsWith(`${c.dir}/dist`)) continue;
    done.add(rel);
    tasks.reads++;
    const read = path.relative(path.join(proj, c.dir), s.path);
    const by = selectedBy(entry, rel);
    if (VERBOSE) console.error(`[${c.name}] reads ${rel}: ${by ?? 'not selected'}`);
    if (by) {
      tasks.selected++;
      if (!selectedBy(entry, rel, NO_RUNNER)) tasks.needRunner++;
    } else {
      missed++;
      if (expectedHole(c)) tasks.holes++;
      else {
        total.findings++;
        findings.push({ program: c.name, read, kind: s.kind, what: 'a change to it does not select the consumer of the task\'s output' });
      }
    }
    // The same path in the consumer's package: the script does not run there.
    if (rel.startsWith(c.dir + '/')) {
      const twin = c.consumer + rel.slice(c.dir.length);
      tasks.twins++;
      const twinBy = selectedBy(entry, twin);
      // A reference decides some twins whoever runs the script: the consumer's own
      // manifest, a file that a pattern of the script's read expands to.
      if (twinBy === 'named') {
        total.findings++;
        findings.push({ program: c.name, read, kind: s.kind, what: `the consumer is selected through a name for the same path in its own package, ${twin}, which the script does not read` });
      } else if (twinBy) tasks.twinsByReference++;
      if (selectedBy(entry, twin, NO_RUNNER) === 'named') tasks.twinsBefore++;
    }
  }
  if (expectedHole(c) && missed === 0 && r.seen.length > 0) coveredHoles.push(c.name);
}

// ---- root cases ----
// `reads`: paths the programs read (and the files their children's command lines name);
// `precise`: programs whose use of the directory reads nothing else below it; `needRule`:
// of those, the ones where an unread file below it selects through a name once the
// consumers are switched off.
const roots = { programs: rootCases.length, reads: 0, selected: 0, precise: 0, needRule: 0 };
const UNREAD = 'other/notes.txt';
const RULES_OFF = { args: ['--graph', graphOff] };
for (const c of rootCases) {
  const r = run(c);
  if (r.crashed) {
    total.crashed++;
    if (VERBOSE) console.error(`[${c.name}] crashed: ${r.stderr.trim().split('\n').slice(-3).join(' | ')}`);
  }
  const entry = `${c.dir}/entry.js`;
  const done = new Set();
  const paths = r.seen
    .filter((s) => s.path.startsWith(proj + path.sep))
    .map((s) => ({ rel: path.relative(proj, s.kind === 'dir' ? path.join(s.path, 'added-later.bin') : s.path), kind: s.kind }))
    .concat((c.expect ?? []).map((rel) => ({ rel, kind: 'command' })));
  for (const s of paths) {
    if (done.has(s.rel)) continue;
    done.add(s.rel);
    roots.reads++;
    const by = selectedBy(entry, s.rel);
    if (VERBOSE) console.error(`[${c.name}] reads ${s.rel}: ${by ?? 'not selected'}`);
    if (by) roots.selected++;
    else {
      total.findings++;
      findings.push({ program: c.name, read: s.rel, kind: s.kind });
    }
  }
  if (c.everything) continue;
  roots.precise++;
  if (done.has(UNREAD)) {
    total.findings++;
    findings.push({ program: c.name, read: UNREAD, kind: 'file', what: 'the program reads the file that stands for what it does not read' });
    continue;
  }
  const by = selectedBy(entry, UNREAD);
  if (by) {
    total.findings++;
    findings.push({ program: c.name, read: UNREAD, kind: 'file', what: `the entry is selected (${by}) for a file below the located directory that the program does not read` });
  }
  if (selectedBy(entry, UNREAD, RULES_OFF) === 'named') roots.needRule++;
  else {
    total.findings++;
    findings.push({ program: c.name, read: UNREAD, kind: 'file', what: `with ${ROOT_RULES_OFF} switched off the unread file does not select through a name: the case does not depend on the rule` });
  }
}
if (!flag('--keep')) fs.rmSync(work, { recursive: true, force: true });
else console.error(`names-oracle: kept ${work}`);

for (const f of findings) console.log(`FINDING ${f.program}: reads ${f.read} (${f.kind}); ${f.what ?? 'a change to it does not select the entry'}`);
if (coveredHoles.length) console.log(`documented holes that are covered here (${coveredHoles.length}): ${coveredHoles.join(', ')}`);
console.log(
  `names-oracle: ${total.programs} programs, ${total.reads} paths read: ${total.named} selected through a name, ` +
    `${total.reference} through a reference or listing, ${total.open} through open uncertainty, ` +
    `${total.holes} in documented holes (not selected), ${total.findings} FINDINGS; ${total.crashed} programs crashed`,
);
console.log(
  `names-oracle: assumption cases (they pin the package assumption, they prove nothing): ${assumption.programs} programs, ` +
    `${assumption.reads} paths read in another package: ${assumption.notSelected} not selected by default, ` +
    `${assumption.selectedAnyway.length} selected anyway; without the assumption all selected except ${assumption.holes} in documented holes; with --strict all selected`,
);
console.log(
  `names-oracle: task cases (a build script run by its package's task, consumed from another package): ${tasks.programs} programs, ` +
    `${tasks.reads} paths read: ${tasks.selected} select the consumer (${tasks.needRunner} only because the task names them), ` +
    `${tasks.holes} in documented holes; ${tasks.twins} same paths in the consumer's package: none selects it through a name ` +
    `(${tasks.twinsBefore} did with TURBO_TRACE_DEBUG_NAMES_TASK_RUNNER=0; ${tasks.twinsByReference} through a reference)`,
);
console.log(
  `names-oracle: root cases (a located directory that is measured against, printed, a child's working directory or an argument of a local function): ` +
    `${roots.programs} programs, ${roots.reads} paths read or named by a command: ${roots.selected} select the entry; ` +
    `${roots.precise} programs read nothing else below the directory: an unread file there selects none of them ` +
    `(${roots.needRule} of them through a name with ${ROOT_RULES_OFF} switched off)`,
);
if (VERBOSE) console.log(`by cause: ${JSON.stringify(byCause)}`);
if (VERBOSE && assumption.selectedAnyway.length) console.log(`assumption cases selected anyway: ${assumption.selectedAnyway.join('; ')}`);
const jsonOut = opt('--json', '');
if (jsonOut) {
  fs.mkdirSync(path.dirname(path.resolve(jsonOut)), { recursive: true });
  fs.writeFileSync(jsonOut, JSON.stringify({ total, byCause, findings, coveredHoles, assumption, tasks, roots }, null, 1));
}
process.exit(total.findings > 0 || total.crashed > 0 ? 1 : 0);
