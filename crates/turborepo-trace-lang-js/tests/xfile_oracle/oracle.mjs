#!/usr/bin/env node
// Runtime oracle for cross-file binding (crates/lang-js/src/xfile, crates/cli/src/xfile.rs).
//
// The claim under test: if a test entry E, run under Node, loads the project file T through
// any chain of `require`, `import()` and static imports, then
// `turbotrace affected --tests jest --changed T` lists E. Binding moves the dependencies of
// a module load whose path is a parameter from the helper to the files that supply the
// value and drops the helper's open record; a value it loses on the way is a test that is
// not selected.
//
// For every program (the hand-written projects in cases/ and the ones gen.mjs generates from
// a seed) the oracle
//   1. writes the project to a temporary directory (a small monorepo: a package per
//      directory of pkgs/, each with a jest.config.js, workspace packages linked into
//      node_modules);
//   2. runs every `*.test.js` entry under Node with preload.cjs, with the working directory
//      of its package (as jest does) and what the package's jest.config.js says about
//      `setupFiles` (loaded before the entry) and `moduleNameMapper` (applied to every
//      specifier), once per input vector (the environment bits FLOW_K0..FLOW_K3) when the
//      program reads them, and collects the project files the process loaded;
//   3. runs `turbotrace affected --tests jest --changed <file>` for every file of the
//      project, with binding (TURBO_TRACE_DEBUG_BIND=1, which is also the default) and with
//      TURBO_TRACE_DEBUG_BIND=0.
// A pair (entry, loaded file) that the default run does not select is a FINDING. When the
// run without binding does not select it either it is a BASELINE finding: a problem of the
// generator, the harness, or of the graph without binding. (A problem of the graph without
// binding is a FINDING for an entry that the run without binding selects for another
// reason, the open record of another helper: triage.mjs shows the sinks to tell them
// apart.)
//
// The usefulness signals: the number of programs in which binding selects fewer entries
// than the baseline for at least one changed file, and the number in which the binder bound
// a value or closed a site at all (TURBO_TRACE_DEBUG_BIND_DEBUG=summary; an open record of another
// helper on the same tests hides a narrowed selection). When they are low, the programs do
// not reach the binder.
//
// With --refresh the oracle also checks the graph after an edit: it copies the entry with
// the most loaded files to a new test file next to it (a new caller with the same
// arguments) and asks again for every file that entry loaded. The copy loads the same files,
// so it has to be selected wherever the original is. When it is not, the oracle asks once
// more with a graph built from scratch: a miss that goes away is a REFRESH finding
// (something computed before the edit was reused); one that stays is the same problem as a
// FINDING of the program (a second caller with the same arguments) and is not counted.
//
//   node oracle.mjs --turbotrace <bin> [--gen N] [--seed S] [--only TEXT] [--no-cases]
//                   [--refresh] [--json FILE] [--keep] [--show] [--shapes] [--jobs N] [-v]
//
// --only filters by program name or shape tag. --keep leaves the projects with findings on
// disk (and with -v every project). --show prints the sources of the programs with findings
// (with --only: of every program). --shapes prints the counts per shape tag. Exit status 1
// when there are findings of any kind. The turbotrace tooling builds the binary and runs
// this script; `node triage.mjs <report>` groups the findings of its report.
import { execFile } from 'node:child_process';
import fs from 'node:fs';
import { createRequire } from 'node:module';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { generate } from './gen.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const opt = (name, dflt) => {
  const i = args.indexOf(name);
  return i >= 0 && i + 1 < args.length ? args[i + 1] : dflt;
};

const BIN = path.resolve(opt('--turbotrace', 'target/debug/turbotrace'));
const GEN = Number(opt('--gen', '100'));
const SEED = Number(opt('--seed', '1'));
const ONLY = opt('--only', '');
const JOBS = Number(opt('--jobs', String(Math.max(1, os.availableParallelism() - 1))));
const VERBOSE = flag('-v');
const KEEP = flag('--keep');
const MASKS = ['0000', '1111', '0101', '1010'];
const SOURCE = /\.(js|cjs|mjs|json)$/;

const work = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'xfile-oracle-')));

function walk(dir, rel = '') {
  const out = [];
  for (const e of fs.readdirSync(path.join(dir, rel), { withFileTypes: true })) {
    const r = rel ? `${rel}/${e.name}` : e.name;
    if (e.isDirectory()) out.push(...walk(dir, r));
    else out.push(r);
  }
  return out;
}

function loadPrograms() {
  const out = [];
  if (!flag('--no-cases')) {
    const dir = path.join(HERE, 'cases');
    for (const name of fs.readdirSync(dir).sort()) {
      if (!fs.statSync(path.join(dir, name)).isDirectory()) continue;
      const files = {};
      for (const rel of walk(path.join(dir, name))) files[rel] = fs.readFileSync(path.join(dir, name, rel), 'utf8');
      out.push({ name, files, shapes: [name.split('_')[0]], hand: true });
    }
  }
  for (let i = 0; i < GEN; i++) {
    const g = generate(SEED * 100003 + i);
    out.push({ name: `gen_${SEED}_${i}`, files: g.files, entries: g.entries, shapes: g.shapes, hand: false });
  }
  return out.filter((p) => !ONLY || p.name.includes(ONLY) || p.shapes.includes(ONLY));
}

// The project on disk. A hand-written case only has its sources: the manifests, the jest
// configurations and a README are added here.
function materialize(p) {
  const root = path.join(work, p.name);
  const files = { ...p.files };
  const pkgs = new Set();
  for (const rel of Object.keys(files)) {
    const m = /^(pkgs\/[^/]+)\//.exec(rel);
    if (m) pkgs.add(m[1]);
  }
  files['package.json'] ??= '{"name":"root","version":"1.0.0","private":true,"workspaces":["pkgs/*"]}\n';
  files['README.md'] ??= 'x\n';
  // (A case with a jest configuration at the root has no other: its tests run there.)
  const rootConfig = 'jest.config.js' in files;
  for (const dir of pkgs) {
    files[`${dir}/package.json`] ??= `{"name":"${path.basename(dir)}","version":"1.0.0"}\n`;
    if (!rootConfig) files[`${dir}/jest.config.js`] ??= 'module.exports = {};\n';
  }
  for (const [rel, text] of Object.entries(files)) {
    const f = path.join(root, rel);
    fs.mkdirSync(path.dirname(f), { recursive: true });
    fs.writeFileSync(f, text);
  }
  // What a package manager does for a workspace.
  fs.mkdirSync(path.join(root, 'node_modules'), { recursive: true });
  for (const dir of pkgs) {
    const name = JSON.parse(files[`${dir}/package.json`]).name;
    fs.symlinkSync(path.join('..', dir), path.join(root, 'node_modules', name), 'dir');
  }
  const entries = p.entries ?? Object.keys(files).filter((f) => /\.test\.(js|cjs|mjs)$/.test(f)).sort();
  // Everything a change can be made to (the entries too: a change to one selects it).
  const sources = Object.keys(files).filter((f) => SOURCE.test(f) && !/(^|\/)(package\.json|jest\.config\.js)$/.test(f)).sort();
  const masks = Object.values(files).some((t) => t.includes('FLOW_K')) ? MASKS : [MASKS[0]];
  return { root, entries, sources, masks, files };
}

function exec(cmd, argv, options) {
  return new Promise((resolve) => {
    execFile(cmd, argv, { encoding: 'utf8', maxBuffer: 64 << 20, ...options }, (error, stdout, stderr) => resolve({ error, stdout, stderr }));
  });
}

// The directory an entry runs in: its package (the nearest jest.config.js).
function cwdOf(root, entry, files) {
  let dir = path.posix.dirname(entry);
  while (dir !== '.' && !(`${dir}/jest.config.js` in files)) dir = path.posix.dirname(dir);
  return dir === '.' ? root : path.join(root, dir);
}

const requireConfig = createRequire(import.meta.url);

// What the jest configuration of the entry's package asks the runner to do.
function jestConfig(m, entry) {
  const dir = cwdOf(m.root, entry, m.files);
  let config = {};
  try {
    config = requireConfig(path.join(dir, 'jest.config.js'));
  } catch {}
  const abs = (s) => s.replace('<rootDir>', dir);
  return {
    dir,
    setup: [...(config.setupFiles ?? []), ...(config.setupFilesAfterEnv ?? [])].map(abs),
    mapper: Object.entries(config.moduleNameMapper ?? {}).map(([pattern, target]) => [pattern, abs(target)]),
  };
}

async function runEntry(m, entry, mask, n) {
  const outFile = `${m.root}.seen-${n}.json`;
  const jest = jestConfig(m, entry);
  const env = { PATH: process.env.PATH, HOME: process.env.HOME, XFILE_ORACLE_ROOT: m.root, XFILE_ORACLE_OUT: outFile, XFILE_ORACLE_MAPPER: JSON.stringify(jest.mapper) };
  [...mask].forEach((b, i) => (env[`FLOW_K${i}`] = b));
  const preloads = [path.join(HERE, 'preload.cjs'), ...jest.setup].flatMap((f) => ['--require', f]);
  const r = await exec(process.execPath, [...preloads, '--no-warnings', path.join(m.root, entry)], {
    cwd: jest.dir,
    env,
    timeout: 30000,
  });
  let seen = { loaded: [], errors: ['no output'] };
  try {
    seen = JSON.parse(fs.readFileSync(outFile, 'utf8'));
  } catch {}
  fs.rmSync(outFile, { force: true });
  if (r.error) seen.errors.push(`exit: ${String(r.error.message).split('\n')[0]} ${(r.stderr || '').trim().split('\n').slice(-2).join(' | ')}`);
  return seen;
}

async function affected(m, changed, bind) {
  const env = { ...process.env };
  delete env.TURBO_TRACE_DEBUG_BIND_DEBUG;
  if (bind) env.TURBO_TRACE_DEBUG_BIND = '1';
  else env.TURBO_TRACE_DEBUG_BIND = '0';
  const r = await exec(BIN, ['affected', '--tests', 'jest', '--changed', changed], { cwd: m.root, env });
  if (r.error) return { error: `${String(r.error.message).split('\n')[0]} ${(r.stderr || '').trim().split('\n').slice(-2).join(' | ')}` };
  return { selected: r.stdout.split('\n').filter(Boolean) };
}

// What the binder did: one line per site file, and for a program with findings the sinks,
// the bound dependencies and the closed sites of every site file.
async function bindSummary(m, detail) {
  const run = async (part) => {
    const r = await exec(BIN, ['affected', '--tests', 'jest', '--changed', 'README.md'], { cwd: m.root, env: { ...process.env, TURBO_TRACE_DEBUG_BIND: '1', TURBO_TRACE_DEBUG_BIND_DEBUG: part } });
    return (r.stderr || '').trim().split('\n').filter(Boolean);
  };
  const out = (await run('summary')).filter((l) => l.startsWith('site ')).slice(0, 20);
  if (detail) out.push(...(await run('/')).filter((l) => /^(== facts|sink |dep |closed )/.test(l)).slice(0, 60));
  return out;
}

// A new caller after the graph was built: a copy of the entry that loaded the most files.
async function refreshCheck(m, loaders, res) {
  const count = new Map();
  for (const es of loaders.values()) for (const e of es) count.set(e, (count.get(e) ?? 0) + 1);
  const [entry] = [...count].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1))[0] ?? [];
  if (!entry) return;
  const copy = path.posix.join(path.posix.dirname(entry), 'zz-copy.test.js');
  fs.writeFileSync(path.join(m.root, copy), m.files[entry]);
  res.refresh = [];
  const missed = [];
  for (const [file, es] of loaders) {
    if (!es.has(entry) || file === entry) continue;
    const on = await affected(m, file, true);
    if (on.error) {
      res.error = `affected --changed ${file} after an edit: ${on.error}`;
      return;
    }
    // Where the original is not selected the pair is a finding already.
    if (on.selected.includes(entry) && !on.selected.includes(copy)) missed.push(file);
  }
  if (!missed.length) return;
  // The same questions to a graph built from scratch.
  fs.rmSync(path.join(m.root, '.turbotrace'), { recursive: true, force: true });
  for (const file of missed) {
    const fresh = await affected(m, file, true);
    if (!fresh.error && fresh.selected.includes(copy)) res.refresh.push({ entry: copy, file });
  }
}

async function check(p) {
  const res = { name: p.name, shapes: p.shapes, pairs: 0, selected: 0, findings: [], baseline: [], narrowed: 0, widened: 0, crashed: false, errors: [], error: null };
  const m = materialize(p);
  res.root = m.root;
  // file -> the entries that loaded it
  const loaders = new Map();
  let n = 0;
  for (const entry of m.entries) {
    for (const mask of m.masks) {
      const seen = await runEntry(m, entry, mask, n++);
      if (seen.errors.length) {
        res.crashed = true;
        for (const e of seen.errors) res.errors.push(`${entry} [${mask}]: ${e}`);
      }
      for (const f of seen.loaded) {
        if (!loaders.has(f)) loaders.set(f, new Set());
        loaders.get(f).add(entry);
      }
    }
  }
  res.loaded = Object.fromEntries([...loaders].map(([f, es]) => [f, [...es].sort()]));
  const changes = [...new Set([...m.sources, ...loaders.keys(), 'README.md'])].sort();
  for (const changed of changes) {
    const on = await affected(m, changed, true);
    const off = await affected(m, changed, false);
    if (on.error || off.error) {
      res.error = `affected --changed ${changed}: ${on.error || off.error}`;
      break;
    }
    const onSet = new Set(on.selected);
    const offSet = new Set(off.selected);
    if (on.selected.some((e) => !offSet.has(e))) res.widened++;
    else if (onSet.size < offSet.size) res.narrowed++;
    for (const entry of loaders.get(changed) ?? []) {
      if (entry === changed) continue;
      res.pairs++;
      if (onSet.has(entry)) res.selected++;
      else if (offSet.has(entry)) res.findings.push({ entry, file: changed });
      else res.baseline.push({ entry, file: changed });
    }
  }
  if (flag('--refresh') && !res.error) await refreshCheck(m, loaders, res);
  res.bind = await bindSummary(m, res.findings.length > 0);
  // site <file>: N known, M open, K closed
  res.bound = res.bind.some((l) => /^site .*: (?!0 known, \d+ open, 0 closed)/.test(l));
  if (res.findings.length || res.baseline.length || res.refresh?.length || res.error || flag('--show')) res.files = m.files;
  const keep = KEEP && (VERBOSE || res.findings.length || res.baseline.length || res.refresh?.length || res.error);
  if (!keep) fs.rmSync(m.root, { recursive: true, force: true });
  else res.kept = m.root;
  return res;
}

const programs = loadPrograms();
const results = new Array(programs.length);
let next = 0;
let done = 0;
await Promise.all(
  Array.from({ length: Math.min(JOBS, programs.length) }, async () => {
    while (next < programs.length) {
      const i = next++;
      try {
        results[i] = await check(programs[i]);
      } catch (e) {
        results[i] = { name: programs[i].name, shapes: programs[i].shapes, pairs: 0, selected: 0, findings: [], baseline: [], narrowed: 0, widened: 0, bound: false, crashed: false, errors: [], error: `oracle: ${e.stack}` };
      }
      if (++done % 50 === 0) console.error(`xfile-oracle: ${done}/${programs.length} programs`);
    }
  }),
);

const total = { programs: results.length, pairs: 0, selected: 0, findings: 0, baseline: 0, refresh: 0, narrowedPrograms: 0, widenedPrograms: 0, boundPrograms: 0, crashed: 0, errors: 0 };
// Per shape tag: programs, programs where binding narrowed a selection, findings.
const byShape = {};
for (const r of results) {
  total.pairs += r.pairs;
  total.selected += r.selected;
  total.findings += r.findings.length;
  total.baseline += r.baseline.length;
  total.refresh += r.refresh?.length ?? 0;
  total.narrowedPrograms += r.narrowed > 0 ? 1 : 0;
  total.widenedPrograms += r.widened > 0 ? 1 : 0;
  total.boundPrograms += r.bound ? 1 : 0;
  total.crashed += r.crashed ? 1 : 0;
  total.errors += r.error ? 1 : 0;
  for (const s of r.shapes) {
    const b = (byShape[s] ??= { programs: 0, narrowed: 0, bound: 0, crashed: 0, withFindings: 0, withBaseline: 0 });
    b.programs++;
    b.narrowed += r.narrowed > 0 ? 1 : 0;
    b.bound += r.bound ? 1 : 0;
    b.crashed += r.crashed ? 1 : 0;
    b.withFindings += r.findings.length ? 1 : 0;
    b.withBaseline += r.baseline.length ? 1 : 0;
  }
}
for (const r of results) {
  const tag = `${r.name} [${r.shapes.join(', ')}]`;
  if (r.error) console.log(`ERROR ${tag}: ${r.error}`);
  for (const f of r.findings) console.log(`FINDING ${tag}: ${f.entry} loads ${f.file}; not selected with binding (selected with TURBO_TRACE_DEBUG_BIND=0)`);
  for (const f of r.baseline) console.log(`BASELINE ${tag}: ${f.entry} loads ${f.file}; not selected with or without binding`);
  for (const f of r.refresh ?? []) console.log(`REFRESH ${tag}: ${f.entry} (a copy of an entry, added after the first queries) loads ${f.file}; not selected after the edit, selected with a graph built from scratch`);
  if ((r.findings.length || r.baseline.length || r.error) && r.kept) console.log(`    kept ${r.kept}`);
  if (r.findings.length && r.bind && !VERBOSE) console.log(`    ${r.bind.join('\n    ')}`);
  if (flag('--show') && (ONLY || r.findings.length || r.baseline.length)) {
    for (const [f, text] of Object.entries(r.files ?? {}).sort()) {
      if (/(^|\/)(package\.json|jest\.config\.js|README\.md)$/.test(f)) continue;
      console.log(`  --- ${f}\n${text.replace(/^/gm, '  | ').replace(/\n  \| $/, '')}`);
    }
    for (const [f, es] of Object.entries(r.loaded ?? {}).sort()) console.log(`  loaded ${f}: ${es.join(', ')}`);
  }
  if (VERBOSE) {
    for (const e of r.errors) console.log(`crashed ${tag}: ${e}`);
    console.log(`  ${tag}: ${r.pairs} pairs, narrowed for ${r.narrowed} files, widened for ${r.widened}${r.bind?.length ? `\n    ${r.bind.join('\n    ')}` : ''}`);
  }
}
if (VERBOSE || flag('--shapes')) {
  for (const [s, b] of Object.entries(byShape).sort()) {
    console.log(`  shape ${s}: ${b.programs} programs, binding narrowed ${b.narrowed}, bound or closed ${b.bound}, with findings ${b.withFindings}, with baseline findings ${b.withBaseline}, crashed ${b.crashed}`);
  }
}
console.log(
  `xfile-oracle: ${total.programs} programs, ${total.pairs} (entry, file) pairs: ${total.selected} selected, ${total.findings} FINDINGS, ` +
    `${total.baseline} baseline findings; binding changed the selection in ${total.narrowedPrograms} programs (narrowed; widened in ${total.widenedPrograms}) ` +
    `and bound or closed a site in ${total.boundPrograms}; ` +
    `${total.crashed} crashed, ${total.errors} errors` +
    (flag('--refresh') ? `; ${total.refresh} REFRESH findings (a new caller after the first queries)` : ''),
);
if (KEEP) console.error(`xfile-oracle: kept projects below ${work}`);
else fs.rmSync(work, { recursive: true, force: true });
const jsonOut = opt('--json', '');
if (jsonOut) {
  fs.mkdirSync(path.dirname(path.resolve(jsonOut)), { recursive: true });
  const interesting = results.filter((r) => r.findings.length || r.baseline.length || r.refresh?.length || r.error || r.crashed);
  fs.writeFileSync(jsonOut, JSON.stringify({ seed: SEED, total, byShape, results: interesting }, null, 1));
}
process.exit(total.findings > 0 || total.baseline > 0 || total.refresh > 0 || total.errors > 0 ? 1 : 0);
