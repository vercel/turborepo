#!/usr/bin/env node
// Runtime oracle for the JS/TS value-flow engine (crates/lang-js/src/flow).
//
// For every program (the hand-written ones in cases/ and the ones gen.mjs generates from a
// seed) it
//   1. runs `extract --format json` on the file, and
//   2. runs the file under Node with instrument.cjs preloaded, once per input vector (the
//      environment bits FLOW_K0..FLOW_K3 that programs use as opaque conditions), recording
//      the first argument of every tracked file API, `require` and `import()` call with the
//      line of the call.
// A recorded value is a FINDING when no reference of the file covers it and the site (the
// same line) has no open uncertainty record: the engine claimed a value set smaller than
// what the program produces, so `affected` would skip a test.
//
// Not findings (the engine's documented assumptions):
//   - values that are not strings (undefined, null, booleans, numbers);
//   - paths below the temporary directory ($TMPDIR of the program);
//   - the empty string;
//   - a name with `undefined` or `null` in it (a path built from a missing value), and a
//     site whose line carries `/* assumed: <which assumption> */`, counted as "assumed".
//
// References next to a resolved module (base `require.resolve`: `require.resolve('m')/x`,
// `import.meta.resolve('m')/../y`, `require.resolve('m', from <start>)`) are not text a site
// receives: the oracle finds the module with Node itself, from the place the reference names
// (locate() below, independent of the trace's resolver), and the reference then stands for
// that absolute path. The project has several copies of one package (next to the program's
// package, next to a sibling directory, inside a dependency), so a reference resolved from
// the wrong place names another file. A call through a function made by `createRequire` is
// recorded with the file Node resolved; it is covered by a reference that names that file
// (a bare reference is resolved from the program, a path with `require`'s extension search).
// For such programs the oracle also runs `trace` and requires every file a
// covered load resolved to in the project to be in the trace: that checks the resolver.
// `partial_specifier` records are not open (they are informational); `cwd_dependent` ones
// are not either: the reference next to them has to cover the value relative to the
// working directory, because per-test working directories resolve those records.
//
// A MODULE THAT IS NOT INSTALLED when the graph is built has cases of its own
// (notinstalled.mjs, names `notinstalled.<case>`): there the engine assumes that a load
// next to a registry package stays in that package's tree, and the cases where the load
// really leaves it at run time are assumption cases, counted apart in every run. They run
// with the hand-written cases (not with --no-cases, --flow-off or --decline).
//
//   node oracle.mjs --turbotrace <bin> [--gen N] [--seed S] [--only TEXT] [--no-cases]
//                   [--file-level] [--flow-off | --decline] [--json FILE] [--keep] [-v]
//
// --flow-off runs the extraction with TURBO_TRACE_DEBUG_FLOW=0, --decline makes the engine decline
// every program: both check that what the token evaluator claims alone is sound too.
//
// Exit status 1 when there are findings (or, with --strict-crash, crashing hand-written
// cases). The trace's development tooling builds the binary and runs this script.
import { execFileSync, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { generate } from './gen.mjs';
import { runNotInstalled } from './notinstalled.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const opt = (name, dflt) => {
  const i = args.indexOf(name);
  return i >= 0 && i + 1 < args.length ? args[i + 1] : dflt;
};

const BIN = path.resolve(opt('--turbotrace', 'target/debug/turbotrace'));
const GEN = Number(opt('--gen', '300'));
const SEED = Number(opt('--seed', '1'));
const ONLY = opt('--only', '');
const VERBOSE = flag('-v');
const MASKS = ['0000', '1111', '0101', '1010', '0011', '1100'];
// Uncertainty kinds that keep a site open (every test reaching it is selected).
const INFORMATIONAL = new Set(['partial_specifier', 'cwd_dependent']);
const MODULE_KINDS = new Set(['require', 'dynamic_import', 'import', 're_export']);

const work = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'flow-oracle-')));
const proj = path.join(work, 'proj');
const srcDir = path.join(proj, 'pkg', 'src');
const tmpDir = path.join(work, 'tmp');
fs.mkdirSync(path.join(srcDir, 'fx', 'sub'), { recursive: true });
fs.mkdirSync(tmpDir, { recursive: true });
for (const f of ['fx/a.json', 'fx/b.json', 'fx/sub/c.json']) fs.writeFileSync(path.join(srcDir, f), '{"file":"a.json"}');
fs.writeFileSync(path.join(proj, 'pkg', 'package.json'), '{"name":"pkg"}');
// Packages for the references next to a resolved module. `fo-dep` exists four times: a
// search from the wrong place finds another copy. `fo-eve` has different `import` and
// `require` entries in different directories: the wrong resolver finds another directory.
const put = (rel, text) => {
  const f = path.join(proj, rel);
  fs.mkdirSync(path.dirname(f), { recursive: true });
  fs.writeFileSync(f, text);
};
for (const [dir, tag] of [['node_modules', 'root'], ['pkg/node_modules', 'pkg'], ['other/node_modules', 'other'], ['node_modules/fo-eve/node_modules', 'eve']]) {
  put(`${dir}/fo-dep/package.json`, '{"name":"fo-dep","main":"index.js"}');
  put(`${dir}/fo-dep/index.js`, `module.exports = ${JSON.stringify(tag)};`);
}
put('package.json', '{"name":"root"}');
put('other/package.json', '{"name":"other"}');
put('other/x.js', 'module.exports = "other x";');
put('pkg/src/fx/own.cjs', 'module.exports = "own";');
put('node_modules/fo-eve/package.json', '{"name":"fo-eve","exports":{".":{"import":"./dist/index.mjs","require":"./cjs/index.cjs"},"./package.json":"./package.json"}}');
put('node_modules/fo-eve/dist/index.mjs', 'export default "esm";');
put('node_modules/fo-eve/dist/inner.js', 'module.exports = "dist inner";');
put('node_modules/fo-eve/dist/deep/leaf.js', 'module.exports = "leaf";');
put('node_modules/fo-eve/cjs/index.cjs', 'module.exports = "cjs";');
put('node_modules/fo-eve/cjs/inner.js', 'module.exports = "cjs inner";');

function loadCases() {
  const out = [];
  if (!flag('--no-cases')) {
    const dir = path.join(HERE, 'cases');
    for (const name of fs.readdirSync(dir).sort()) {
      if (/\.(cjs|mjs|ts|mts|cts|js)$/.test(name)) out.push({ name, text: fs.readFileSync(path.join(dir, name), 'utf8'), hand: true });
    }
  }
  for (let i = 0; i < GEN; i++) out.push({ name: `gen_${SEED}_${i}.cjs`, text: generate(SEED * 100003 + i), hand: false });
  return out.filter((c) => !ONLY || c.name.includes(ONLY));
}

function extract(file) {
  // --flow-off: the token evaluator alone (tier 1), as when the engine is switched off.
  const env = flag('--flow-off') ? { ...process.env, TURBO_TRACE_DEBUG_FLOW: '0' } : process.env;
  const out = execFileSync(BIN, ['extract', '--format', 'json', file], { encoding: 'utf8', maxBuffer: 64 << 20, env });
  return JSON.parse(out).refs;
}

function run(file, mask) {
  const outFile = path.join(work, 'seen.json');
  fs.rmSync(outFile, { force: true });
  const env = { PATH: process.env.PATH, HOME: process.env.HOME, TMPDIR: tmpDir, FLOW_ORACLE_FILE: file, FLOW_ORACLE_OUT: outFile, FLOW_PRESENT: 'env-value', FLOW_NESTED: 'n1/n2', FLOW_ABS: '/abs/elsewhere.json' };
  [...mask].forEach((b, i) => (env[`FLOW_K${i}`] = b));
  const nodeArgs = ['--require', path.join(HERE, 'instrument.cjs'), '--no-warnings'];
  if (/\.(ts|mts|cts)$/.test(file)) nodeArgs.push('--experimental-transform-types');
  const r = spawnSync(process.execPath, [...nodeArgs, file, 'argv-value'], { cwd: proj, env, encoding: 'utf8', timeout: 20000 });
  let seen = [];
  try {
    seen = JSON.parse(fs.readFileSync(outFile, 'utf8'));
  } catch {}
  const crashed = r.status !== 0 || seen.some((s) => s.kind === 'crash');
  return { seen: seen.filter((s) => s.kind !== 'crash'), crashed, stderr: r.stderr || '' };
}

const rx = (spec) => new RegExp('^' + spec.split('*').map((s) => s.replace(/[.+?^${}()|[\]\\]/g, '\\$&')).join('[^]*') + '$');
const norm = (p) => (p.length > 1 && p.endsWith('/') ? p.replace(/\/+$/, '') : p);

// The file a located module is (the text form of a reference next to a resolved module),
// found by Node: `require.resolve` from the program's directory or from the start the text
// names, `import.meta.resolve` in a child process whose module lives in that directory.
// Returns [file, rest of the text] or null.
const esmCache = new Map();
function locate(text, fileDir) {
  const m = /^(require\.resolve|import\.meta\.resolve)\('([^']*)'/.exec(text);
  if (!m) return null;
  let rest = text.slice(m[0].length);
  let from = fileDir;
  if (rest.startsWith(', from ')) {
    rest = rest.slice(', from '.length);
    if (rest.startsWith("'")) {
      const end = rest.indexOf("'", 1);
      from = path.resolve(fileDir, rest.slice(1, end));
      rest = rest.slice(end + 1);
    } else {
      const inner = locate(rest, fileDir);
      if (!inner) return null;
      [from, rest] = inner;
      if (rest.startsWith("/'")) {
        const end = rest.indexOf("'", 2);
        from = path.resolve(from, rest.slice(2, end));
        rest = rest.slice(end + 1);
      }
    }
    // Node searches from a file as from its directory.
    if (fs.existsSync(from) && fs.statSync(from).isFile()) from = path.dirname(from);
  }
  if (!rest.startsWith(')')) return null;
  rest = rest.slice(1);
  try {
    if (m[1] === 'require.resolve') return [createRequire(path.join(from, 'noop.js')).resolve(m[2]), rest];
    const key = `${from}\0${m[2]}`;
    if (!esmCache.has(key)) {
      const r = spawnSync(process.execPath, ['--input-type=module', '-e', `process.stdout.write(import.meta.resolve(${JSON.stringify(m[2])}))`], { cwd: from, encoding: 'utf8' });
      esmCache.set(key, r.status === 0 && r.stdout.startsWith('file:') ? fileURLToPath(r.stdout) : null);
    }
    const file = esmCache.get(key);
    return file ? [file, rest] : null;
  } catch {
    return null;
  }
}

// The strings a reference stands for, as regexes over absolute paths or bare specifiers.
function refMatchers(ref, fileDir) {
  if (ref.spec === '<unknown>') return [];
  const out = [];
  for (let alt of ref.spec.split(' | ')) {
    if (ref.base === 'require.resolve') {
      // Next to a resolved module: the path joined onto the file Node finds.
      const found = locate(alt, fileDir);
      if (found && (found[1] === '' || found[1].startsWith('/'))) {
        const tail = MODULE_KINDS.has(ref.kind) ? found[1].replace(/[?#].*$/s, '') : found[1];
        out.push({ bare: false, located: true, search: ref.kind === 'require', re: rx(norm(path.posix.normalize(found[0] + tail))) });
      }
      continue;
    }
    // The resolver ignores the query and fragment of a module specifier.
    if (MODULE_KINDS.has(ref.kind) && /^(\.|\/)/.test(alt)) alt = alt.replace(/[?#].*$/s, '');
    const relative = alt.startsWith('./') || alt.startsWith('../') || alt === '.' || alt === '..';
    const bareModule = MODULE_KINDS.has(ref.kind) && !relative && !alt.startsWith('/');
    if (ref.base !== 'file_dir' && ref.base !== 'cwd') {
      out.push({ bare: true, re: rx(alt) });
    } else if (bareModule) {
      out.push({ bare: true, spec: alt.includes('*') ? null : alt, re: rx(alt) });
    } else {
      const base = ref.base === 'cwd' ? proj : fileDir;
      out.push({ bare: false, search: ref.kind === 'require', re: rx(norm(path.posix.normalize(alt.startsWith('/') ? alt : base + '/' + alt))) });
    }
  }
  return out;
}

// Whether a reference covers the file `abs` that a function made by `createRequire` resolved:
// a path reference names it (with the extension and index search of `require`), a bare
// reference resolves to it from the program.
const SEARCH = ['', '.js', '.cjs', '.json', '/index.js', '/index.cjs'];
function coversResolved(m, abs, file) {
  if (!m.bare) return m.re.test(abs) || (m.search && SEARCH.some((s) => s && abs.endsWith(s) && m.re.test(abs.slice(0, -s.length))));
  if (!m.spec) return false;
  try {
    return createRequire(file).resolve(m.spec) === abs;
  } catch {
    return false;
  }
}

// The files `trace` reaches from `file`, as absolute paths.
function traced(file) {
  try {
    const out = execFileSync(BIN, ['trace', '--format', 'list', '--base', proj, '--cwd', proj, file], { encoding: 'utf8', maxBuffer: 64 << 20 });
    return new Set(out.split('\n').filter(Boolean).map((l) => path.resolve(proj, l)));
  } catch (e) {
    return null;
  }
}

// --decline: every program ends with a direct `eval` that never runs, so the value-flow
// engine declines the file and the token results stand (the same happens for files that are
// too deep or do not parse).
const DECLINE = "\nif (process.env.FLOW_NEVER) eval('0');\n";

function check(c) {
  const file = path.join(srcDir, c.name);
  if (flag('--decline')) c = { ...c, text: c.text + DECLINE };
  fs.writeFileSync(file, c.text);
  const res = { name: c.name, sites: 0, covered: 0, open: 0, exempt: 0, assumed: 0, findings: [], crashed: false, error: null };
  let refs;
  try {
    refs = extract(file);
  } catch (e) {
    res.error = `extract failed: ${String(e.message).split('\n')[0]}`;
    return res;
  }
  // Site-level by default: only the references the site itself produced (same line, not the
  // `asset` references of anchored path expressions, which come from another mechanism and
  // would hide a wrong site value). `--file-level` accepts any reference of the file.
  const fileLevel = flag('--file-level');
  const all = refs.map((r) => ({ line: r.line, asset: r.kind === 'asset', ms: refMatchers(r, srcDir) }));
  const matchersFor = (line) => all.filter((r) => fileLevel || line === 0 || (r.line === line && !r.asset)).flatMap((r) => r.ms);
  const openLines = new Set(refs.filter((r) => r.uncertainty && !INFORMATIONAL.has(r.uncertainty.kind)).map((r) => r.line));
  const anyOpenImport = refs.some((r) => r.kind === 'dynamic_import' && r.uncertainty && !INFORMATIONAL.has(r.uncertainty.kind));
  // Programs with references next to a resolved module also have their resolution checked.
  let trace;
  const located = refs.some((r) => r.base === 'require.resolve');
  const inTrace = (abs) => {
    if (!located || !abs.startsWith(proj + '/') || !fs.existsSync(abs) || !fs.statSync(abs).isFile()) return true;
    if (trace === undefined) trace = traced(file);
    return trace === null || trace.has(fs.realpathSync(abs)) || trace.has(abs);
  };
  const done = new Set();
  for (const mask of MASKS) {
    const r = run(file, mask);
    if (r.crashed) res.crashed = true;
    if (VERBOSE && r.stderr.trim()) console.error(`[${c.name} ${mask}] ${r.stderr.trim().split('\n')[0]}`);
    for (const s of r.seen) {
      const key = `${s.line}\0${s.kind}\0${s.t}\0${s.v}`;
      if (done.has(key)) continue;
      done.add(key);
      res.sites++;
      if (s.t !== 'string' || s.v === '') {
        res.exempt++;
        continue;
      }
      const moduleLike = s.kind === 'require' || s.kind === 'dynamic_import';
      let v = s.v;
      // `import()` loads the file whatever the query or fragment says.
      if (s.kind === 'dynamic_import' && /^(\.|\/|file:)/.test(v)) v = v.replace(/[?#].*$/s, '');
      if (moduleLike && v.startsWith('file:')) v = fileURLToPath(v);
      if (moduleLike && /^(node:|data:)/.test(v)) continue;
      if (moduleLike && !v.startsWith('.') && !v.startsWith('/') && isBuiltin(v)) continue;
      const bare = moduleLike && !v.startsWith('./') && !v.startsWith('../') && !v.startsWith('/') && v !== '.' && v !== '..';
      const abs = bare ? v : norm(path.resolve(moduleLike ? srcDir : proj, v));
      if (!bare && (abs === tmpDir || abs.startsWith(tmpDir + '/'))) {
        res.exempt++;
        continue;
      }
      const ms = matchersFor(s.line);
      // A call through a function made by `createRequire` recorded the file it resolved.
      const covered = s.resolved ? ms.some((m) => coversResolved(m, abs, file)) : ms.some((m) => m.bare === bare && (m.re.test(abs) || (m.located && m.search && SEARCH.some((x) => x && abs.endsWith(x) && m.re.test(abs.slice(0, -x.length))))));
      if (covered) {
        if (!bare && moduleLike && !inTrace(abs)) {
          res.findings.push({ line: s.line, api: `${s.api} (trace)`, value: path.relative(srcDir, abs), source: c.text.split('\n')[s.line - 1]?.trim() ?? '', refs: ['covered by a reference, but `turbotrace trace` does not reach the file'] });
          continue;
        }
        res.covered++;
        continue;
      }
      if (openLines.has(s.line) || (s.line === 0 && anyOpenImport)) {
        res.open++;
        continue;
      }
      // A site the case marks with `/* assumed: … */` relies on a documented assumption.
      if (/undefined|null/.test(v) || /\/\* assumed: /.test(c.text.split('\n')[s.line - 1] ?? '')) {
        res.assumed++;
        continue;
      }
      const here = refs.filter((x) => x.line === s.line).map((x) => `${x.kind} ${x.base}:${x.spec}${x.uncertainty ? ' !' + x.uncertainty.kind : ''}`);
      res.findings.push({ line: s.line, api: s.api, value: bare ? v : path.relative(srcDir, abs), source: c.text.split('\n')[s.line - 1]?.trim() ?? '', refs: here });
    }
  }
  return res;
}

function isBuiltin(v) {
  return ['fs', 'path', 'os', 'url', 'module', 'util', 'child_process', 'worker_threads', 'fs/promises', 'assert', 'events', 'crypto', 'process'].includes(v);
}

const cases = loadCases();
const results = [];
let n = 0;
for (const c of cases) {
  results.push(check(c));
  if (++n % 100 === 0) console.error(`flow-oracle: ${n}/${cases.length} programs`);
}
if (!flag('--keep')) fs.rmSync(work, { recursive: true, force: true });
else console.error(`flow-oracle: kept ${work}`);
// The cases of a module that is not installed (see notinstalled.mjs): the located modules
// are evaluated by the value-flow engine, so not without it.
const notInstalled =
  flag('--no-cases') || flag('--flow-off') || flag('--decline')
    ? null
    : runNotInstalled({ bin: BIN, only: ONLY, verbose: VERBOSE, keep: flag('--keep') });

const total = { programs: results.length, sites: 0, covered: 0, open: 0, exempt: 0, assumed: 0, findings: 0, crashed: 0, errors: 0 };
for (const r of results) {
  total.sites += r.sites;
  total.covered += r.covered;
  total.open += r.open;
  total.exempt += r.exempt;
  total.assumed += r.assumed;
  total.findings += r.findings.length;
  total.crashed += r.crashed ? 1 : 0;
  total.errors += r.error ? 1 : 0;
}
for (const r of results) {
  if (r.error) console.log(`ERROR ${r.name}: ${r.error}`);
  for (const f of r.findings) {
    console.log(`FINDING ${r.name}:${f.line} ${f.api} got ${JSON.stringify(f.value)}; engine: ${f.refs.length ? f.refs.join(', ') : '(nothing)'}\n    ${f.source}`);
  }
  if (VERBOSE && r.crashed) console.log(`crashed ${r.name}`);
}
console.log(
  `flow-oracle: ${total.programs} programs, ${total.sites} observed (site, value) pairs: ${total.covered} covered by a reference, ` +
    `${total.open} at an open site, ${total.exempt} exempt, ${total.assumed} assumed, ${total.findings} FINDINGS; ${total.crashed} programs crashed, ${total.errors} extract errors`,
);
let notInstalledFindings = 0;
if (notInstalled && notInstalled.count.cases) {
  const n = notInstalled.count;
  notInstalledFindings = notInstalled.findings.length;
  console.log(
    `flow-oracle, modules that are not installed: ${n.cases} case(s), ${n.pairs} (program, loaded file) pair(s): ${n.inside} inside the package tree of the registry package (the lockfile selects the program), ` +
      `${n.covered} file(s) of the repository selected, ${n.assumed} ASSUMPTION pair(s) (the load leaves the package tree: not selected by default, selected with --strict and with TURBO_TRACE_DEBUG_LOCATED_PACKAGE=0), ` +
      `${n.controls} control(s); ${notInstalledFindings} FINDING(S)`,
  );
  for (const f of notInstalled.findings) console.log(`  FINDING ${f}`);
}
const jsonOut = opt('--json', '');
if (jsonOut) {
  fs.mkdirSync(path.dirname(path.resolve(jsonOut)), { recursive: true });
  const report = { total, results: results.filter((r) => r.findings.length || r.error) };
  if (notInstalled) report.notInstalled = { counts: notInstalled.count, findings: notInstalled.findings, cases: notInstalled.rows };
  fs.writeFileSync(jsonOut, JSON.stringify(report, null, 1));
}
process.exit(total.findings > 0 || total.errors > 0 || notInstalledFindings > 0 ? 1 : 0);
