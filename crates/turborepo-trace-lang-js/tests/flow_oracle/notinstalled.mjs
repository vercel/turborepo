// The flow oracle's cases for a load next to a module that is NOT INSTALLED when the graph
// is built (crates/lang-js/src/resolve/located.rs, "A module that is not installed").
//
// A load can say where it resolves from: `createRequire(require.resolve('pkg'))('./x.js')`,
// a path joined next to `require.resolve('pkg/package.json')`, a module searched from
// there. In a checkout without `node_modules` the trace cannot find the file. It links the
// module through the lockfile, and for a package of the registry it ASSUMES that the load
// stays inside the package tree that lockfile entry stands for (the package and what it
// installs), as everything a registry package loads by itself does. The record is covered:
// only a change of the lockfile entry selects the test. `--strict` counts the load as
// open, `TURBO_TRACE_DEBUG_LOCATED_PACKAGE=0` turns the assumption off, `--explain` states it.
//
// The assumption carries a lot (with it off, the mean selection of one large repository
// more than doubles), so these cases pin what it says and what it gives up. One pnpm-style
// workspace is written WITHOUT an install and its graph is built (twice: by default, and
// with `TURBO_TRACE_DEBUG_LOCATED_PACKAGE=0`). Then the packages are installed the way pnpm does
// it (a virtual store, one link per declared dependency, the hidden hoist of every
// workspace package for the code inside the store), every case's program runs under Node
// with a preload that records each file it loads, and `affected` is asked about
// the graph of the uninstalled tree.
//
// Kinds of case:
//
//   inside      the load stays in the tree of the registry package: every file it brings in
//               is a file of that package or of a package it installs. Nothing of the
//               repository is loaded, and a change of the lockfile selects the program.
//               A file of the repository loaded by such a case and not selected is a
//               FINDING.
//   assumption  the load LEAVES the tree at run time and brings in a file of the
//               repository: a module the registry package's own search finds through the
//               hidden hoist (a workspace package, as a plugin host finds a plugin), or a
//               path next to the package with enough parent steps to climb out of the
//               store. The assumption says this does not happen. These cases prove
//               nothing: they are what the assumption gives up, counted apart in every
//               run. Not selected by default, selected with `--strict` and with
//               `TURBO_TRACE_DEBUG_LOCATED_PACKAGE=0`; anything else is a FINDING (also "selected
//               by default": the list is kept honest). A case that loses its kind here
//               (`assumption` -> `inside`) is a finding of the first sort.
//
// Controls, for every case: a change to a file of the workspace package that no program
// loads must not select the program by default (the assumption bounds something), and must
// select it with `--strict` and with the switch off (without the assumption the site is a
// load with an unknown target, which reaches the packages its package declares: the case
// exercises the assumption, not an edge).
//
// `runNotInstalled({ bin, only, verbose, keep })` returns the counts and the findings.
import { execFileSync, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const PRELOAD = path.join(HERE, 'notinstalled-preload.cjs');

// The virtual store: where the registry package and its dependency are installed.
const STORE = 'node_modules/.pnpm';
const REG = `${STORE}/fo-reg@1.0.0/node_modules/fo-reg`;
const REG_DEP = `${STORE}/fo-reg-dep@1.0.0/node_modules/fo-reg-dep`;
// The package tree the lockfile entry of `fo-reg` stands for.
const TREE = [`${STORE}/fo-reg@1.0.0/`, `${STORE}/fo-reg-dep@1.0.0/`];
const PLUG = 'packages/plug';
const CONTROL = `${PLUG}/control.js`;
const LOCKFILE = 'pnpm-lock.yaml';

const CJS_HEAD = "const { createRequire } = require('node:module');\nconst path = require('node:path');\n";
const MJS_HEAD = "import { createRequire } from 'node:module';\nimport path from 'node:path';\nconst require = createRequire(import.meta.url);\n";
// From the package directory in the store up to the workspace root.
const UP = "'..', '..', '..', '..', '..'";

// body: the statement with the load; loads: the files it must bring in (checked: a case
// that does not do what it says is a finding).
const CASES = {
  // ---- the load stays inside the tree ----
  nextToTheEntry: {
    kind: 'inside',
    body: "createRequire(require.resolve('fo-reg'))('./inner.js');",
    loads: [`${REG}/inner.js`],
  },
  nextToTheManifest: {
    kind: 'inside',
    body: "require(path.join(path.dirname(require.resolve('fo-reg/package.json')), 'lib', 'x.js'));",
    loads: [`${REG}/lib/x.js`],
  },
  itsOwnDependency: {
    kind: 'inside',
    body: "createRequire(require.resolve('fo-reg'))('fo-reg-dep');",
    loads: [`${REG_DEP}/index.js`],
  },
  byUrl: {
    kind: 'inside',
    esm: true,
    body: "await import(new URL('./esm.mjs', import.meta.resolve('fo-reg')).href);",
    loads: [`${REG}/esm.mjs`],
  },
  searchedFromAnotherPackage: {
    kind: 'inside',
    body: "createRequire(require.resolve('fo-reg', { paths: [path.join(__dirname, '../../packages/other')] }))('./inner.js');",
    loads: [`${REG}/inner.js`],
  },
  // ---- ASSUMPTION CASES: the load leaves the tree ----
  // The registry package's own search finds a workspace package through the hidden hoist,
  // as a plugin host finds a plugin of the repository.
  aWorkspacePackageThroughTheHoist: {
    kind: 'assumption',
    body: "createRequire(require.resolve('fo-reg'))('@o/plug');",
    loads: [`${PLUG}/index.js`],
  },
  aWorkspacePackageSearchedWithPaths: {
    kind: 'assumption',
    body: "require(require.resolve('@o/plug/data.js', { paths: [path.dirname(require.resolve('fo-reg/package.json'))] }));",
    loads: [`${PLUG}/data.js`],
  },
  // The path next to the package has enough parent steps to leave the store.
  aPathThatClimbsOut: {
    kind: 'assumption',
    body: `require(path.join(path.dirname(require.resolve('fo-reg/package.json')), ${UP}, 'packages', 'plug', 'data.js'));`,
    loads: [`${PLUG}/data.js`],
  },
  aUrlThatClimbsOut: {
    kind: 'assumption',
    esm: true,
    body: "await import(new URL('../../../../../packages/plug/esm.mjs', import.meta.resolve('fo-reg')).href);",
    loads: [`${PLUG}/esm.mjs`],
  },
  searchedFromAnotherPackageAndOut: {
    kind: 'assumption',
    body: "createRequire(require.resolve('fo-reg', { paths: [path.join(__dirname, '../../packages/other')] }))('@o/plug');",
    loads: [`${PLUG}/index.js`],
  },
};

export function runNotInstalled({ bin, only = '', verbose = false, keep = false }) {
  const cases = Object.entries(CASES)
    .map(([id, c]) => ({ id, name: `notinstalled.${id}`, ...c, dir: `services/${id.toLowerCase()}` }))
    .filter((c) => !only || c.name.includes(only))
    .map((c) => ({ ...c, entry: `${c.dir}/main.${c.esm ? 'mjs' : 'cjs'}` }));
  const count = { cases: 0, pairs: 0, inside: 0, covered: 0, assumed: 0, controls: 0 };
  const findings = [];
  const rows = [];
  if (!cases.length) return { count, findings, rows };

  // ---- the workspace, without an install ----
  const work = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'flow-oracle-notinstalled-')));
  const ws = path.join(work, 'ws');
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
  write('.gitignore', 'node_modules\n');
  write('package.json', JSON.stringify({ name: 'root', private: true }));
  write('pnpm-workspace.yaml', "packages:\n  - 'packages/*'\n  - 'services/*'\n");
  write(`${PLUG}/package.json`, JSON.stringify({ name: '@o/plug', version: '1.0.0', main: 'index.js' }));
  write(`${PLUG}/index.js`, "module.exports = 'a plugin of the repository';\n");
  write(`${PLUG}/data.js`, "module.exports = 'data of the repository';\n");
  write(`${PLUG}/esm.mjs`, "export default 'a module of the repository';\n");
  write(CONTROL, "module.exports = 'no program loads this';\n");
  write('packages/other/package.json', JSON.stringify({ name: '@o/other', version: '1.0.0', dependencies: { 'fo-reg': '^1' } }));
  write('packages/other/index.js', "module.exports = 'other';\n");
  const importer = (deps) => `    dependencies:\n${deps}`;
  const regDep = "      fo-reg:\n        specifier: ^1\n        version: 1.0.0\n";
  const plugDep = "      '@o/plug':\n        specifier: workspace:*\n        version: link:../../packages/plug\n";
  let lock = `lockfileVersion: '9.0'\nimporters:\n  .: {}\n  packages/other:\n${importer(regDep)}  packages/plug: {}\n`;
  for (const c of [...cases].sort((a, b) => (a.dir < b.dir ? -1 : 1))) {
    write(`${c.dir}/package.json`, JSON.stringify({ name: `svc-${c.id.toLowerCase()}`, version: '1.0.0', dependencies: { '@o/plug': 'workspace:*', 'fo-reg': '^1' } }));
    write(c.entry, `${c.esm ? MJS_HEAD : CJS_HEAD}${c.body}\n`);
    lock += `  ${c.dir}:\n${importer(plugDep + regDep)}`;
  }
  lock +=
    'packages:\n  fo-reg-dep@1.0.0:\n    resolution: {integrity: sha512-dep}\n  fo-reg@1.0.0:\n    resolution: {integrity: sha512-reg}\n' +
    'snapshots:\n  fo-reg-dep@1.0.0: {}\n  fo-reg@1.0.0:\n    dependencies:\n      fo-reg-dep: 1.0.0\n';
  write(LOCKFILE, lock);

  // ---- the graphs of the tree as it is checked out: nothing installed ----
  const graphs = { default: path.join(work, 'graph'), off: path.join(work, 'graph-off') };
  const envFor = (how) => {
    const env = { ...process.env };
    delete env.TURBO_TRACE_DEBUG_LOCATED_PACKAGE;
    if (how === 'off') env.TURBO_TRACE_DEBUG_LOCATED_PACKAGE = '0';
    return env;
  };
  for (const how of ['default', 'off']) {
    execFileSync(bin, ['graph', 'build', '--root', ws, '--graph', graphs[how]], { cwd: ws, stdio: ['ignore', 'ignore', 'inherit'], env: envFor(how) });
  }

  // ---- the install, as pnpm lays it out ----
  write(`${REG}/package.json`, JSON.stringify({ name: 'fo-reg', version: '1.0.0', main: 'index.js', dependencies: { 'fo-reg-dep': '^1' } }));
  write(`${REG}/index.js`, "module.exports = 'fo-reg';\n");
  write(`${REG}/inner.js`, "module.exports = 'inner';\n");
  write(`${REG}/lib/x.js`, "module.exports = 'x';\n");
  write(`${REG}/esm.mjs`, "export default 'esm';\n");
  write(`${REG_DEP}/package.json`, JSON.stringify({ name: 'fo-reg-dep', version: '1.0.0', main: 'index.js' }));
  write(`${REG_DEP}/index.js`, "module.exports = 'dep';\n");
  link(`${STORE}/fo-reg@1.0.0/node_modules/fo-reg-dep`, REG_DEP);
  // The hidden hoist: every workspace package, for the code inside the store.
  link(`${STORE}/node_modules/@o/plug`, PLUG);
  link(`${STORE}/node_modules/@o/other`, 'packages/other');
  link('packages/other/node_modules/fo-reg', REG);
  for (const c of cases) {
    link(`${c.dir}/node_modules/fo-reg`, REG);
    link(`${c.dir}/node_modules/@o/plug`, PLUG);
  }

  function run(c) {
    const outFile = path.join(work, `seen-${c.id}.json`);
    const env = { PATH: process.env.PATH, HOME: process.env.HOME, NOTINSTALLED_OUT: outFile };
    const r = spawnSync(process.execPath, ['--require', PRELOAD, '--no-warnings', path.join(ws, c.entry)], { cwd: path.join(ws, c.dir), env, encoding: 'utf8', timeout: 30000 });
    let seen = { loaded: [], errors: ['no output'] };
    try {
      seen = JSON.parse(fs.readFileSync(outFile, 'utf8'));
    } catch {}
    fs.rmSync(outFile, { force: true });
    if (r.status !== 0) seen.errors.push(`exit ${r.status}: ${(r.stderr || '').trim().split('\n').slice(-2).join(' | ')}`);
    return seen;
  }

  // Whether a change of `rel` selects `entry`, in the graph of the uninstalled tree.
  const memo = new Map();
  function selected(entry, rel, how) {
    const key = `${how}\0${entry}\0${rel}`;
    if (!memo.has(key)) {
      const graph = how === 'off' ? graphs.off : graphs.default;
      const argv = ['affected', entry, '--changed', rel, '--root', ws, '--graph', graph, '--no-refresh', '--format', 'json'];
      if (how === 'strict') argv.push('--strict');
      const out = execFileSync(bin, argv, { cwd: ws, encoding: 'utf8', maxBuffer: 16 << 20, env: envFor(how) });
      memo.set(key, JSON.parse(out).results.some((e) => e.entry === entry));
    }
    return memo.get(key);
  }

  for (const c of cases) {
    count.cases++;
    const seen = run(c);
    // Files of the workspace: what the program loaded besides itself, by where it is.
    const files = seen.loaded.filter((f) => f.startsWith(ws + path.sep)).map((f) => path.relative(ws, f).split(path.sep).join('/'));
    const loaded = files.filter((f) => f !== c.entry);
    const row = { name: c.name, kind: c.kind, loaded, errors: seen.errors, pairs: [] };
    rows.push(row);
    const finding = (what) => findings.push(`${c.name} (${c.kind}): ${what}`);
    if (seen.errors.length) finding(`the program failed: ${seen.errors.join('; ')}`);
    for (const f of c.loads) if (!loaded.includes(f)) finding(`the case does not load ${f} (loaded: ${loaded.join(', ') || 'nothing'})`);
    for (const f of loaded) {
      count.pairs++;
      if (TREE.some((t) => f.startsWith(t))) {
        // Inside the tree of the registry package: the lockfile entry stands for it.
        const dflt = selected(c.entry, LOCKFILE, 'default');
        row.pairs.push({ file: f, where: 'tree', lockfile: dflt });
        if (dflt) count.inside++;
        else finding(`${f} is a file of the package tree, and a change of ${LOCKFILE} does not select the program`);
        continue;
      }
      if (f.includes('/node_modules/')) {
        finding(`${f} is an installed file outside the package tree of fo-reg`);
        continue;
      }
      // A file of the repository.
      const dflt = selected(c.entry, f, 'default');
      const off = selected(c.entry, f, 'off');
      const strict = selected(c.entry, f, 'strict');
      row.pairs.push({ file: f, where: 'repository', default: dflt, off, strict });
      if (c.kind === 'assumption' && c.loads.includes(f)) {
        count.assumed++;
        if (dflt) finding(`${f} is selected by default: the case does not pin the assumption`);
        if (!off) finding(`${f} is not selected with TURBO_TRACE_DEBUG_LOCATED_PACKAGE=0`);
        if (!strict) finding(`${f} is not selected with --strict`);
      } else if (dflt) {
        count.covered++;
      } else {
        finding(`${f} is loaded and not selected: the load left the package tree of fo-reg${off ? ' (selected without the assumption)' : ' (not selected without the assumption either)'}`);
      }
    }
    count.controls++;
    if (selected(c.entry, CONTROL, 'default')) finding(`a change to ${CONTROL}, which the program does not load, selects it by default: the assumption bounds nothing here`);
    if (!selected(c.entry, CONTROL, 'off')) finding(`with TURBO_TRACE_DEBUG_LOCATED_PACKAGE=0 a change to ${CONTROL} does not select the program: the case does not exercise the assumption`);
    if (!selected(c.entry, CONTROL, 'strict')) finding(`--strict does not open the load (a change to ${CONTROL})`);
    if (verbose) console.log(`${c.name} (${c.kind}): ${row.pairs.map((p) => (p.where === 'tree' ? `${p.file}=tree` : `${p.file}=${p.default ? 'selected' : 'no'}/off:${p.off ? 'selected' : 'no'}/strict:${p.strict ? 'selected' : 'no'}`)).join(' ') || '(nothing loaded)'}`);
  }
  if (!keep) fs.rmSync(work, { recursive: true, force: true });
  else console.error(`flow-oracle: kept ${work}`);
  return { count, findings, rows };
}
