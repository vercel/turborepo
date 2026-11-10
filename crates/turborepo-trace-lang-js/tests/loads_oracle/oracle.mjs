#!/usr/bin/env node
// Runtime oracle for bounded loads (crates/query/src/loads.rs, crates/cli/src/loads.rs).
//
// The claim under test: in a workspace with a strict `node_modules` layout, a module load
// whose specifier is computed can only bring in a file of the package of the loading file,
// of a package that package (or a package around it, the root included) declares, or of a
// package nested in one of those; and by induction everything a chain of such loads brings
// in is covered. So for every project file a test really loads,
// `turbotrace affected --tests jest --changed <file>` must select the test.
//
// The oracle writes one pnpm-style workspace:
//
//   <package>/node_modules/@o/<dep>          one link per DECLARED dependency, nothing else
//   node_modules/@o/rootdep                  the root's dependency: resolves from every package
//   node_modules/@o/x-prettier-config        a workspace package matched by a public hoist pattern
//   node_modules/.pnpm/node_modules/@o/*     pnpm's hidden hoist of EVERY workspace package
//                                            (hoist-workspace-packages): only code inside the
//                                            virtual store sees it, i.e. third-party packages
//   node_modules/.pnpm/tp-loader@1.0.0/…     a third-party loader, linked into the harness
//
// and one service per case. Each case's test computes a specifier at run time (text that no
// analysis follows: a decoded constant) and hands it to a load site. The test runs under
// Node with the preload of the cross-file oracle (../xfile_oracle/preload.cjs), which
// records every project file the process loads.
//
// Kinds of case:
//
//   theorem     the load is resolved by the module system from the loading file (or from
//               the test's package). Every loaded file must select the test: anything else
//               is a FINDING. One of them (suppliedValue) has its record on the test that
//               passes the value, where cross-file binding puts it.
//   assumption  the load is by a computed PATH into a package nobody declares, or into the
//               root package's own files. The bound assumes this does not happen. These
//               cases prove nothing: they pin the boundary. Not selected by default,
//               selected with --strict and with TURBO_TRACE_DEBUG_LOADS_PACKAGE=0; anything else is
//               a FINDING.
//   located     the same target, but the code says where the path goes (built on
//               __dirname): an edge, selected by default.
//   environment the same target again, named through an environment variable that the
//               test assigns and a file of the service loads, asked of a `require`
//               function made by `createRequire` (for the file itself, a file beside it, a
//               known place elsewhere, a place that is not known). The environment registry
//               says where the path goes: an absolute value names the same module for every
//               such function, a relative one is searched from the function's place. An
//               edge, selected by default: without it the open load is bounded by the
//               declared packages and the module in the undeclared package is lost. One
//               case is a theorem instead (a relative value asked of a function whose place
//               is not known): nothing may be guessed from the file, the load stays open.
//   floor       a load that turbotrace does not record (the handoff to a third-party loader
//               below), in a test that also reaches a recorded load. Without the bound the
//               floor selected that test on every change and so covered the unrecorded load
//               by accident; with the bound it does not. Counted apart as LOST WITH THE
//               FLOOR in every run: it is what the bound gives up, and it must stay visible.
//               A FINDING if it is not selected with --strict or with the switch off, or if
//               it turns out covered (the list is kept honest).
//   outside     the specifier is handed to a third-party loader, which resolves from its own
//               place in the virtual store and so finds every workspace package through the
//               hidden hoist. turbotrace records no load for it, with or without the bound
//               (the test loads no file with a load site of its own): counted as BASELINE
//               (not selected either way), a FINDING only if the bound loses what an open
//               load selected. A known hole of the load model, not of the bound.
//
// Controls keep the oracle honest. For every case with a load site, a change to a file of
// the undeclared package that the test does NOT load must not select the test by default
// (the bound bounds something) and must select it with TURBO_TRACE_DEBUG_LOADS_PACKAGE=0 (without
// the bound the site is an open load: the case exercises the bound, not an edge).
//
//   node oracle.mjs --turbotrace <bin> [--only TEXT] [--keep] [-v] [--json FILE]
//
// Exit status 1 when there are findings. The turbotrace tooling builds the binary and
// runs this script.
import { execFileSync, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const PRELOAD = path.join(HERE, '..', 'xfile_oracle', 'preload.cjs');
const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const opt = (name, dflt) => {
  const i = args.indexOf(name);
  return i >= 0 && i + 1 < args.length ? args[i + 1] : dflt;
};
const BIN = path.resolve(opt('--turbotrace', 'target/debug/turbotrace'));
const ONLY = opt('--only', '');
const VERBOSE = flag('-v');

// ---- the workspace ----

const ws = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'loads-oracle-')));
const write = (rel, text) => {
  const p = path.join(ws, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, text);
};
const link = (from, to) => {
  const p = path.join(ws, from);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.symlinkSync(path.relative(path.dirname(p), path.join(ws, to)), p);
};
const b64 = (s) => `Buffer.from('${Buffer.from(s).toString('base64')}', 'base64').toString()`;

// name -> [directory, declared workspace packages]
const PACKAGES = {
  harness: ['packages/harness', ['dep']],
  'harness-esm': ['packages/harness-esm', ['dep']],
  dep: ['packages/dep', ['dep2']],
  'dep-nested': ['packages/dep/nested', []],
  dep2: ['packages/dep2', ['trans']],
  trans: ['packages/trans', []],
  rootdep: ['packages/rootdep', []],
  entrydep: ['packages/entrydep', []],
  'x-prettier-config': ['packages/x-prettier-config', []],
  undeclared: ['packages/undeclared', []],
  undeclared2: ['packages/undeclared2', []],
};

write('package.json', JSON.stringify({ name: 'root', private: true, devDependencies: { '@o/rootdep': 'workspace:*' } }));
write('pnpm-workspace.yaml', "packages:\n  - 'packages/*'\n  - 'packages/dep/nested'\n  - 'services/*'\n");
write('tools/rootfile.js', "module.exports = 'a file of the root package';\n");
for (const [name, [dir, deps]] of Object.entries(PACKAGES)) {
  write(`${dir}/package.json`, JSON.stringify({ name: `@o/${name}`, version: '1.0.0', dependencies: Object.fromEntries(deps.map((d) => [`@o/${d}`, 'workspace:*'])) }));
  for (const d of deps) link(`${dir}/node_modules/@o/${d}`, PACKAGES[d][0]);
  // pnpm's hidden hoist: every workspace package, for the code inside the virtual store.
  link(`node_modules/.pnpm/node_modules/@o/${name}`, dir);
}
link('node_modules/@o/rootdep', PACKAGES.rootdep[0]);
link('node_modules/@o/x-prettier-config', PACKAGES['x-prettier-config'][0]);

// The load sites.
write(
  'packages/harness/index.js',
  `exports.load = function (spec) {
  return require(spec);
};
exports.loadAsync = function (spec) {
  return import(spec);
};
exports.loadFrom = function (base, spec) {
  return require('node:module').createRequire(base)(spec);
};
exports.viaThirdParty = function (spec) {
  return require('tp-loader').load(spec);
};
`,
);
// A file without a load of its own: the specifier goes to a third-party loader.
write(
  'packages/harness/third.js',
  `exports.viaThirdParty = function (spec) {
  return require('tp-loader').load(spec);
};
`,
);
write('packages/harness/lib/own.js', "module.exports = 'own';\n");
// The same load site in a file with ES module syntax, imported by name: cross-file binding
// follows the argument to its callers, so the record of a value it cannot evaluate sits on
// the TEST ("a value passed to packages/harness-esm/index.js:2"), not on this file. The
// module system still resolves the specifier from here. A package of its own, with no
// other load site: a load of the harness that the change can reach would make every file
// that reaches it affected, and the test's package with them.
write(
  'packages/harness-esm/index.js',
  `export function load(spec) {
  return require(spec);
}
`,
);
const TP = 'node_modules/.pnpm/tp-loader@1.0.0/node_modules/tp-loader';
write(`${TP}/package.json`, JSON.stringify({ name: 'tp-loader', version: '1.0.0', main: 'index.js' }));
write(`${TP}/index.js`, 'exports.load = function (spec) {\n  return require(spec);\n};\n');
link('packages/harness/node_modules/tp-loader', TP);

// What the loads bring in.
write('packages/dep/index.js', "exports.name = 'dep';\nexports.a = require('./lib/a');\n");
write('packages/dep/lib/a.js', "module.exports = 'a';\n");
write('packages/dep/lib/extra.js', "module.exports = 'extra: a file nothing imports';\n");
write('packages/dep/data.json', '{"data": true}\n');
write('packages/dep/lib/chain.js', 'exports.next = function (spec) {\n  return require(spec);\n};\n');
write('packages/dep/lib/cross.js', "module.exports = require('../../undeclared2/index.js');\n");
write('packages/dep/nested/n.js', "module.exports = 'nested';\n");
write('packages/dep2/index.js', 'exports.next = function (spec) {\n  return require(spec);\n};\n');
write('packages/trans/index.js', "module.exports = 'trans';\n");
write('packages/rootdep/index.js', "module.exports = 'rootdep';\n");
write('packages/entrydep/index.js', "module.exports = 'entrydep';\n");
write('packages/x-prettier-config/index.js', "module.exports = 'public';\n");
write('packages/undeclared/index.js', "module.exports = 'undeclared';\n");
write('packages/undeclared/located.js', "module.exports = 'located';\n");
write('packages/undeclared/control.js', "module.exports = 'no test loads this';\n");
write('packages/undeclared2/index.js', "module.exports = 'reached by a relative import of dep';\n");
for (const n of ['beside', 'let', 'unknown', 'relative', 'cwd']) write(`packages/undeclared/env-${n}.js`, `module.exports = 'named by the environment: ${n}';\n`);

// ---- the cases ----
// body: the statements of the test (H is the harness); loads: files that must be loaded
// (a sanity check of the case); deps: what the service declares next to the harness.
const CASES = {
  declared: { kind: 'theorem', body: `H.load(${b64('@o/dep')});`, loads: ['packages/dep/index.js', 'packages/dep/lib/a.js'] },
  subpath: { kind: 'theorem', body: `H.load(${b64('@o/dep/lib/extra.js')});`, loads: ['packages/dep/lib/extra.js'] },
  json: { kind: 'theorem', body: `H.load(${b64('@o/dep/data.json')});`, loads: ['packages/dep/data.json'] },
  dynamicImport: { kind: 'theorem', body: `H.loadAsync(${b64('@o/dep/lib/extra.js')});`, loads: ['packages/dep/lib/extra.js'] },
  // Three loads, each resolved from the file the previous one brought in.
  chain: {
    kind: 'theorem',
    body: `H.load(${b64('@o/dep/lib/chain.js')}).next(${b64('@o/dep2')}).next(${b64('@o/trans')});`,
    loads: ['packages/dep/lib/chain.js', 'packages/dep2/index.js', 'packages/trans/index.js'],
  },
  rootDependency: { kind: 'theorem', body: `H.load(${b64('@o/rootdep')});`, loads: ['packages/rootdep/index.js'] },
  publicHoist: { kind: 'theorem', body: `H.load(${b64('@o/x-prettier-config')});`, loads: ['packages/x-prettier-config/index.js'] },
  // The loaded file has an ordinary relative import that leaves its package.
  staticCrossing: { kind: 'theorem', body: `H.load(${b64('@o/dep/lib/cross.js')});`, loads: ['packages/dep/lib/cross.js', 'packages/undeclared2/index.js'] },
  nestedPackage: { kind: 'theorem', body: `H.load(${b64('@o/dep/nested/n.js')});`, loads: ['packages/dep/nested/n.js'] },
  ownPackage: { kind: 'theorem', body: `H.load(${b64('./lib/own.js')});`, loads: ['packages/harness/lib/own.js'] },
  // The base of the resolution is the test's file: the test's package declares the target.
  entryBase: { kind: 'theorem', deps: ['entrydep'], body: `H.loadFrom(__filename, ${b64('@o/entrydep')});`, loads: ['packages/entrydep/index.js'] },
  siteInTest: { kind: 'theorem', deps: ['entrydep'], body: `require(${b64('@o/entrydep')});`, loads: ['packages/entrydep/index.js'] },
  // The record sits on the test that supplies the value (see harness-esm above); the target
  // is a package that the loader's package declares and the test's package does not. The
  // test's package declares the loader's package only.
  suppliedValue: { kind: 'theorem', esm: true, deps: ['harness-esm'], body: `load(${b64('@o/dep/lib/extra.js')});`, loads: ['packages/dep/lib/extra.js'] },
  // ASSUMPTION CASES.
  pathIntoUndeclared: { kind: 'assumption', body: `H.load(require('node:path').join(${b64(ws)}, 'packages', 'undeclared', 'index.js'));`, loads: ['packages/undeclared/index.js'] },
  pathIntoRootPackage: { kind: 'assumption', body: `H.load(require('node:path').join(${b64(ws)}, 'tools', 'rootfile.js'));`, loads: ['tools/rootfile.js'] },
  // Evidence beats the assumption: the code locates the path.
  locatedPath: { kind: 'located', body: "require(require('node:path').join(__dirname, '../../../packages/undeclared/located.js'));", loads: ['packages/undeclared/located.js'] },
  // The hidden hoist: a third-party loader finds a package nobody declares.
  // The same handoff from a file that also holds a recorded load (the harness). Without the
  // bound the test was selected on every change, through the floor of that other load, so
  // the floor covered this too. With the bound it is not: what the bound gives up.
  hiddenHoistBesideALoad: { kind: 'floor', body: `H.viaThirdParty(${b64('@o/undeclared')});`, loads: ['packages/undeclared/index.js'] },
  hiddenHoist: { kind: 'outside', body: `require('@o/harness/third.js').viaThirdParty(${b64('@o/undeclared')});`, harness: false, loads: ['packages/undeclared/index.js'] },
  // THE ENVIRONMENT: the test assigns a variable, a file of the service loads what it names.
  // An absolute value asked of a function made for a file beside the loading file.
  envBeside: envCase('BESIDE', "createRequire(path.join(__dirname, 'noop.js'))", 'process.env.ORACLE_ENV_BESIDE', "path.join(__dirname, '../../../packages/undeclared/env-beside.js')", 'packages/undeclared/env-beside.js'),
  // Of a function made for the file itself, bound to a name that can be bound again.
  envRebindable: envCase('LET', 'createRequire(__filename)', 'process.env.ORACLE_ENV_LET', "path.join(__dirname, '../../../packages/undeclared/env-let.js')", 'packages/undeclared/env-let.js', 'let'),
  // Of a function whose place is not known: an absolute path is the same from anywhere.
  envUnknownPlace: envCase('UNKNOWN', "createRequire(path.join(process.cwd(), 'noop.js'))", 'process.env.ORACLE_ENV_UNKNOWN', "path.join(__dirname, '../../../packages/undeclared/env-unknown.js')", 'packages/undeclared/env-unknown.js'),
  // A relative value asked of a function made for a known place in the other package: it
  // is searched there, not from the loading file.
  envElsewhere: envCase('RELATIVE', "createRequire(path.join(__dirname, '../../../packages/undeclared/noop.js'))", 'process.env.ORACLE_ENV_RELATIVE', "'./env-relative.js'", 'packages/undeclared/env-relative.js'),
  // A value resolved against the working directory where it is asked (the test runs in its
  // package, as under jest), of a function whose place is not known.
  envResolvedAtCwd: envCase('CWD', "createRequire(path.join(process.cwd(), 'noop.js'))", 'path.resolve(process.cwd(), process.env.ORACLE_ENV_CWD)', "'../../packages/undeclared/env-cwd.js'", 'packages/undeclared/env-cwd.js'),
  // A relative value asked of a function whose place is not known (at run time the working
  // directory, the service): from the loading file (`src/`) it would name nothing, and a
  // guess would close the load. It stays open: the module, a file of the test's package, is
  // covered by the bound, and the controls see an open load.
  envRelativeUnknownPlace: {
    ...envCase('OPEN', "createRequire(path.join(process.cwd(), 'noop.js'))", 'process.env.ORACLE_ENV_OPEN', "'./lib/env-open.js'", null),
    kind: 'theorem',
    extra: { 'lib/env-open.js': "module.exports = 'a module of the service';\n" },
    loadsOwn: ['lib/env-open.js'],
  },
};

// A case of kind `environment`: `src/load.js` of the service loads `request` through the
// `require` function `made` (bound with `decl`), and the test assigns `value` to the
// variable first. `target`: the file of the undeclared package that is loaded.
function envCase(tag, made, request, value, target, decl = 'const') {
  return {
    kind: 'environment',
    harness: false,
    files: {
      'src/load.js': `const { createRequire } = require('node:module');\nconst path = require('node:path');\n${decl} req = ${made};\nmodule.exports = () => req(${request});\n`,
    },
    body: `const path = require('node:path');\n  process.env.ORACLE_ENV_${tag} = ${value};\n  require('../src/load.js')();`,
    loads: target ? [target] : [],
  };
}

const cases = Object.entries(CASES)
  .filter(([id]) => !ONLY || id.includes(ONLY))
  .map(([id, c]) => ({ id, ...c, dir: `services/${id.toLowerCase()}`, entry: `services/${id.toLowerCase()}/test/case.test.js` }));
for (const c of cases) {
  const deps = [...(c.esm ? [] : ['harness']), ...(c.deps ?? [])];
  write(`${c.dir}/package.json`, JSON.stringify({ name: `svc-${c.id.toLowerCase()}`, version: '1.0.0', dependencies: Object.fromEntries(deps.map((d) => [`@o/${d}`, 'workspace:*'])) }));
  for (const d of deps) link(`${c.dir}/node_modules/@o/${d}`, PACKAGES[d][0]);
  write(`${c.dir}/jest.config.js`, 'module.exports = {};\n');
  const head = c.esm ? "import { load } from '@o/harness-esm';\n" : c.harness === false ? '' : "const H = require('@o/harness');\n";
  write(c.entry, `${head}test('t', () => {\n  ${c.body}\n});\n`);
  for (const [rel, text] of Object.entries({ ...c.files, ...c.extra })) write(`${c.dir}/${rel}`, text);
  // Files of the service itself that the case must load.
  for (const rel of c.loadsOwn ?? []) c.loads = [...c.loads, `${c.dir}/${rel}`];
}

// ---- run ----

execFileSync(BIN, ['graph', 'build', '--root', ws], { cwd: ws, stdio: ['ignore', 'ignore', 'inherit'] });

function run(c) {
  const outFile = path.join(ws, `.seen-${c.id}.json`);
  const env = { PATH: process.env.PATH, HOME: process.env.HOME, XFILE_ORACLE_ROOT: ws, XFILE_ORACLE_OUT: outFile, XFILE_ORACLE_MAPPER: '[]' };
  const r = spawnSync(process.execPath, ['--require', PRELOAD, '--no-warnings', path.join(ws, c.entry)], { cwd: path.join(ws, c.dir), env, encoding: 'utf8', timeout: 30000 });
  let seen = { loaded: [], errors: ['no output'] };
  try {
    seen = JSON.parse(fs.readFileSync(outFile, 'utf8'));
  } catch {}
  fs.rmSync(outFile, { force: true });
  if (r.status !== 0) seen.errors.push(`exit ${r.status}: ${(r.stderr || '').trim().split('\n').slice(-2).join(' | ')}`);
  return seen;
}

const memo = new Map();
function selected(rel, how) {
  const key = `${how}\0${rel}`;
  if (!memo.has(key)) {
    const argv = ['affected', '--tests', 'jest', '--no-refresh', '--root', ws, '--changed', path.join(ws, rel)];
    if (how === 'strict') argv.push('--strict');
    const env = { ...process.env };
    delete env.TURBO_TRACE_DEBUG_LOADS_PACKAGE;
    if (how === 'off') env.TURBO_TRACE_DEBUG_LOADS_PACKAGE = '0';
    const out = execFileSync(BIN, argv, { cwd: ws, encoding: 'utf8', maxBuffer: 16 << 20, env });
    memo.set(key, new Set(out.split('\n').filter(Boolean)));
  }
  return memo.get(key);
}

const CONTROL = 'packages/undeclared/control.js';
const findings = [];
const rows = [];
const count = { cases: 0, pairs: 0, covered: 0, assumed: 0, lostWithFloor: 0, baseline: 0, controls: 0 };
for (const c of cases) {
  count.cases++;
  const seen = run(c);
  const loaded = seen.loaded.filter((f) => f !== c.entry);
  const row = { id: c.id, kind: c.kind, loaded, errors: seen.errors, pairs: [] };
  rows.push(row);
  const finding = (what) => {
    findings.push(`${c.id} (${c.kind}): ${what}`);
  };
  if (seen.errors.length) finding(`the program failed: ${seen.errors.join('; ')}`);
  for (const f of c.loads) if (!loaded.includes(f)) finding(`the case does not load ${f} (loaded: ${loaded.join(', ')})`);
  for (const f of loaded) {
    count.pairs++;
    const dflt = selected(f, 'default').has(c.entry);
    const off = selected(f, 'off').has(c.entry);
    const strict = selected(f, 'strict').has(c.entry);
    row.pairs.push({ file: f, default: dflt, off, strict });
    const target = c.loads.includes(f);
    if (c.kind === 'assumption' && target) {
      count.assumed++;
      if (dflt) finding(`${f} is selected by default: the case does not pin the assumption`);
      if (!off) finding(`${f} is not selected with TURBO_TRACE_DEBUG_LOADS_PACKAGE=0`);
      if (!strict) finding(`${f} is not selected with --strict`);
    } else if (c.kind === 'floor' && target) {
      count.lostWithFloor++;
      if (dflt) finding(`${f} is selected by default: no longer lost with the floor, move the case`);
      if (!off) finding(`${f} is not selected with TURBO_TRACE_DEBUG_LOADS_PACKAGE=0: the floor did not cover it`);
      if (!strict) finding(`${f} is not selected with --strict`);
    } else if (dflt) {
      count.covered++;
    } else if (c.kind === 'outside' && !off) {
      // No load is recorded for a specifier handed to third-party code: not the bound's.
      count.baseline++;
    } else {
      finding(`${f} is loaded and not selected${off ? ' (selected without the bound)' : ' (nor without the bound)'}`);
    }
  }
  if (c.kind === 'theorem' || c.kind === 'assumption' || c.kind === 'floor') {
    count.controls++;
    if (selected(CONTROL, 'default').has(c.entry)) finding(`a change to ${CONTROL}, which the test does not load, selects it: nothing is bounded`);
    if (!selected(CONTROL, 'off').has(c.entry)) finding(`without the bound a change to ${CONTROL} does not select the test: the case has no open load`);
    if (!selected(CONTROL, 'strict').has(c.entry)) finding(`--strict does not open the load (a change to ${CONTROL})`);
  }
  if (VERBOSE) console.log(`${c.id} (${c.kind}): ${row.pairs.map((p) => `${p.file}=${p.default ? 'selected' : 'no'}${p.off ? '' : '/off:no'}`).join(' ')}`);
}

const report = { workspace: ws, counts: count, findings, cases: rows };
const jsonOut = opt('--json', '');
if (jsonOut) {
  fs.mkdirSync(path.dirname(path.resolve(jsonOut)), { recursive: true });
  fs.writeFileSync(jsonOut, JSON.stringify(report, null, 2));
}
console.log(
  `loads oracle: ${count.cases} case(s), ${count.pairs} (test, loaded file) pair(s): ${count.covered} covered, ${count.assumed} assumption pair(s) (not selected by default, selected with --strict and with the switch off), ${count.lostWithFloor} LOST WITH THE FLOOR (an unrecorded load that only the floor of another load selected; selected with --strict and with the switch off), ${count.baseline} baseline (a load turbotrace does not record, with or without the bound), ${count.controls} control(s); ${findings.length} FINDING(S)`,
);
for (const f of findings) console.log(`  FINDING ${f}`);
if (!flag('--keep')) fs.rmSync(ws, { recursive: true, force: true });
else console.log(`kept ${ws}`);
process.exit(findings.length ? 1 : 0);
