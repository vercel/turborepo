#!/usr/bin/env node
// Runtime oracle for mock-aware loading (crates/query/src/mocks.rs,
// crates/frameworks/src/mocks.rs).
//
// The claim under test: a jest or vitest test that replaces a module with a mock does not
// run the module's code, so a change that reaches the test only through the replaced
// module does not select it. Whatever the rule cuts, the opposite must hold: for every
// first-party file whose code runs in the test, `affected --changed <file>`
// selects the test.
//
// Every case is a small project of its own (one jest or vitest project: a config, a module
// graph, one test file) built from a grammar:
//
//   runner   jest with babel-jest, @swc/jest, ts-jest, native ES modules, next/jest behind a
//            transformer file of the repository, such a file around @swc/jest and one
//            that compiles by itself; vitest
//   graph    how the test reaches the module `m`: directly, through a barrel, through an
//            alias, with a second path to m's dependency, with a dynamic import, not at
//            all (a mock of a module nothing requests loads nothing), ...
//   form     how m is replaced: a factory, a factory that loads the real module, a manual
//            mock, an automock, a spy, a late `doMock`, ...
//   where    where the call stands: the top of the test, a setup file, an imported helper,
//            a helper that a setup file imports, a hook, a block, a function
//   neg      what undoes it: requireActual / importActual, unmock, resetModules,
//            isolateModules, a setup file that loads m first, a helper that requests m
//            itself, runner configuration, ...
//   style    how the test loads its subject: `import`, a top-level `require`, a load inside
//            the test body
//
// Observation: every first-party module pushes `<file>@<version>` on a global list when its
// top level runs, and the test writes the list (and the values it saw) to a file. The base
// run has version 1 everywhere. The head run bumps the version in every file at once: a
// file's bump shows in a test's observation exactly when the file ran in it, so one head
// run stands for one single-file mutation per file (`--single N` re-runs N of them alone
// and compares). A file whose mutation changes the observation of a test that
// `affected --changed <file>` does not select is a FINDING ("rule" when the test is selected
// with `TURBO_TRACE_DEBUG_MOCKS=0`, "graph" when it is not selected either way).
//
//   node oracle.mjs --turbotrace <bin> [--gen N] [--seed S] [--only TEXT] [--runners jb,js,jt,je,jw,jx,jn,vt]
//                   [--single N] [--work DIR] [--keep] [-v] [--json FILE]
//
// Needs node and the network (one `npm install` into <work>/node_modules, reused between
// runs). Exit status 1 when there are findings. The trace's development tooling builds the binary
// and runs this script.
import { execFileSync, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const opt = (name, dflt) => {
  const i = args.indexOf(name);
  return i >= 0 && i + 1 < args.length ? args[i + 1] : dflt;
};
const BIN = path.resolve(opt('--turbotrace', 'target/debug/turbotrace'));
const ONLY = opt('--only', '');
const GEN = Number(opt('--gen', '0'));
const SEED = Number(opt('--seed', '1'));
const SINGLE = Number(opt('--single', '0'));
const VERBOSE = flag('-v');
const WORK = path.resolve(opt('--work', 'target/mock-oracle/work'));
const RUNNERS = opt('--runners', 'jb,js,jt,je,jw,jx,jn,vt').split(',');

// ---- runners ----

const BABEL = `['babel-jest', { presets: [['@babel/preset-env', { targets: { node: 'current' } }], '@babel/preset-typescript'] }]`;
const SWC = `['@swc/jest', { jsc: { parser: { syntax: 'typescript' }, target: 'es2022' } }]`;
const TSJEST = `['ts-jest', { isolatedModules: true, diagnostics: false, tsconfig: { module: 'commonjs', target: 'es2022', esModuleInterop: true, allowJs: true } }]`;
const WRAP = `const { transformSync } = require('@babel/core');
const { createTransformer } = require('@swc/jest');
const swc = createTransformer({ jsc: { parser: { syntax: 'typescript' }, target: 'es2022' } });
module.exports = {
  process(source, filename, options) {
    const pre = transformSync(source, { filename, babelrc: false, configFile: false, parserOpts: { plugins: ['typescript'] }, plugins: [] });
    return swc.process(pre.code, filename, options);
  },
};
`;
const OWN = `const { transformSync } = require('@swc/core');
module.exports = {
  process(source, filename) {
    return { code: transformSync(source, { filename, jsc: { parser: { syntax: 'typescript' }, target: 'es2022' }, module: { type: 'commonjs' }, sourceMaps: 'inline' }).code };
  },
};
`;
// `next/jest`, with its SWC transformer wrapped by a file of the repository that runs a
// Babel pass first (the shape of an app that adds a Babel plugin to its tests).
const NEXT_CONFIG = `module.exports = async () => {
  const config = await nextJest({ dir: __dirname })(custom)();
  return {
    ...config,
    transform: Object.fromEntries(
      Object.entries(config.transform ?? {}).map(([pattern, transformer]) => {
        if (Array.isArray(transformer) && transformer[0] === require.resolve('next/dist/build/swc/jest-transformer')) {
          return [pattern, ['<rootDir>/../../nextwrap.cjs', transformer[1]]];
        }
        return [pattern, transformer];
      }),
    ),
  };
};
`;
const NEXTWRAP = `const { transformSync } = require('@babel/core');
const { createTransformer } = require('next/dist/build/swc/jest-transformer');
module.exports = {
  createTransformer(options) {
    const swc = createTransformer(options);
    return {
      process(source, filename, jestOptions) {
        const pre = transformSync(source, { filename, babelrc: false, configFile: false, parserOpts: { plugins: ['typescript', 'jsx'] }, plugins: [] });
        return swc.process(pre.code, filename, jestOptions);
      },
    };
  },
};
`;
const RUNNER = {
  jb: { kind: 'jest', transform: BABEL },
  js: { kind: 'jest', transform: SWC },
  jt: { kind: 'jest', transform: TSJEST },
  // jest with native ES modules (`--experimental-vm-modules`): `jest.mock` is not hoisted
  // above static imports, which load the real module.
  je: { kind: 'jest', transform: SWC, esm: true },
  // A transformer file of the repository: a Babel pass, then @swc/jest (hoists) ...
  jw: { kind: 'jest', transform: "'<rootDir>/../../wrap.cjs'", wrapper: WRAP },
  // ... and one that compiles with @swc/core itself: `jest.mock` stays below the imports.
  jx: { kind: 'jest', transform: "'<rootDir>/../../own.cjs'", wrapper: OWN },
  jn: { kind: 'jest', next: true, wrapper: NEXTWRAP },
  vt: { kind: 'vitest' },
};

// ---- the grammar ----

const GRAPHS = ['direct', 'barrel', 'barrel-whole', 'barrel-other', 'second', 'testimports', 'dynamic', 'typeonly', 'alias', 'mapper-stub', 'deep-target', 'native-require', 'pkg', 'pkg-missing', 'unrequested', 'bound'];
const FORMS = {
  factory: {},
  'factory-ext': {},
  'factory-import': { only: 'vitest' },
  'factory-var': {},
  'factory-fake': {},
  partial: {},
  'partial-actual': {},
  manual: {},
  auto: {},
  spy: { only: 'vitest' },
  virtual: { only: 'jest' },
  domock: {},
  setmock: { only: 'jest' },
  none: {},
};
const WHERES = ['top', 'setup', 'setupAfterEnv', 'helper-before', 'helper-after', 'beforeAll', 'block', 'fn', 'inTest', 'setup-helper', 'setupAfterEnv-helper'];
const NEGS = {
  none: {},
  'actual-in-test': {},
  'actual-in-helper': {},
  'unmock-top': {},
  'unmock-reset': {},
  'dontmock-reset': {},
  'reset-reload': {},
  isolate: { only: 'jest' },
  'domock-override': {},
  'domock-bare': {},
  'load-direct': {},
  'actual-of-user': {},
  'automock-user': {},
  'config-noisolate': { only: 'vitest' },
  'config-globals': { only: 'vitest' },
  'helper-bare-mock': {},
  'config-resetModules': { only: 'jest' },
  'config-automock': { only: 'jest' },
  'setup-loads-m': {},
  'setup-loads-user': {},
  'setup-unmock': {},
  'helper-loads-m': {},
  'setup-helper-late': {},
  'helper-bare-manual': {},
};
const STYLES = { esm: {}, cjs: { only: 'jest' }, dynamic: {} };

const q = (s) => `'${s}'`;
const mark = (rel) => `(globalThis.__loaded ||= []).push('${rel}@1');\n`;

// The runner's spelling of each call.
function api(kind, esm) {
  if (kind === 'jest') {
    return {
      head: esm ? "import { jest } from '@jest/globals';\n" : '',
      mock: (s, f, o) => `jest.mock(${q(s)}${f ? `, ${f}` : ''}${o ? `, ${o}` : ''});`,
      doMock: (s, f) => `jest.doMock(${q(s)}${f ? `, ${f}` : ''});`,
      unmock: (s) => `jest.unmock(${q(s)});`,
      dontMock: (s) => `jest.dontMock(${q(s)});`,
      reset: () => 'jest.resetModules();',
      actual: (s) => `jest.requireActual(${q(s)})`,
      load: (s) => (esm ? `(await import(${q(s)}))` : `require(${q(s)})`),
    };
  }
  return {
    head: "import { vi, test, beforeAll } from 'vitest';\n",
    mock: (s, f, o) => `vi.mock(${q(s)}${f ? `, ${f}` : ''}${o ? `, ${o}` : ''});`,
    doMock: (s, f) => `vi.doMock(${q(s)}${f ? `, ${f}` : ''});`,
    unmock: (s) => `vi.unmock(${q(s)});`,
    dontMock: (s) => `vi.doUnmock(${q(s)});`,
    reset: () => 'vi.resetModules();',
    actual: (s) => `(await vi.importActual(${q(s)}))`,
    load: (s) => `(await import(${q(s)}))`,
  };
}

function valid(spec) {
  const kind = RUNNER[spec.runner].kind;
  for (const [table, key] of [
    [FORMS, spec.form],
    [NEGS, spec.neg],
    [STYLES, spec.style],
  ]) {
    if (table[key].only && table[key].only !== kind) return false;
  }
  if (kind === 'vitest' && spec.where.startsWith('setupAfterEnv')) return false;
  // The setup file imports the subject before the helper that holds the mock.
  if (spec.neg === 'setup-helper-late' && !spec.where.endsWith('-helper')) return false;
  // A manual mock next to the target that a helper registers: a first-party module file.
  if (spec.neg === 'helper-bare-manual' && ['native-require', 'pkg', 'pkg-missing'].includes(spec.graph)) return false;
  // The helper requests m: only where a helper holds the mock, and as an ES import.
  if (spec.neg === 'helper-loads-m' && (!spec.where.includes('helper') || ['native-require', 'pkg-missing'].includes(spec.graph))) return false;
  // Native ES modules in jest: the forms that do not need `require`.
  if (RUNNER[spec.runner].esm) {
    return (
      ['factory', 'manual', 'none'].includes(spec.form) &&
      ['top', 'setup'].includes(spec.where) &&
      spec.neg === 'none' &&
      spec.style !== 'cjs' &&
      !['native-require', 'typeonly', 'pkg-missing'].includes(spec.graph)
    );
  }
  // A mock inside the test body only matters when the subject is loaded after it.
  if (spec.where === 'inTest' && spec.style !== 'dynamic') return false;
  if (spec.form === 'none' && spec.where !== 'top') return false;
  // A package that is declared but not installed only loads as a mock.
  if (spec.graph === 'pkg-missing') {
    return spec.form === (kind === 'jest' ? 'virtual' : 'factory') && ['top', 'setup'].includes(spec.where) && spec.neg === 'none' && !RUNNER[spec.runner].esm && spec.runner !== 'jx';
  }
  // A `mock`-prefixed variable is not initialized yet when the subject's import reads the
  // mocked module's value at load.
  if (kind === 'jest' && spec.form === 'factory-var' && spec.graph === 'deep-target' && spec.style !== 'dynamic') return false;
  // The stub's mapper names the request exactly: no request with an extension.
  if (spec.form === 'factory-ext' && spec.graph === 'mapper-stub') return false;
  return true;
}

// The files of one case (relative to its directory), its config, and the file the mock
// targets.
function build(spec) {
  const R = RUNNER[spec.runner];
  const A = api(R.kind, R.esm);
  // vitest with `globals: true`: `vi` and `test` are not imported.
  if (spec.neg === 'config-globals') A.head = '';
  const jest = R.kind === 'jest';
  const files = {};
  // next/jest wants an app directory in the project.
  if (R.next) files['app/.keep'] = '';
  const target = spec.graph === 'barrel-whole' || spec.graph === 'barrel-other' ? 'barrel' : spec.graph === 'deep-target' ? 'deep' : 'm';
  const cfg = { setup: [], setupAfterEnv: [], alias: null, extra: [], vextra: [], include: 't.test.ts' };
  const native = spec.graph === 'native-require';
  const missing = spec.graph === 'pkg-missing';
  const pkg = spec.graph === 'pkg' || missing;
  const ext = missing ? 'missing-pkg' : 'extpkg';
  const alias = spec.graph === 'alias' || spec.graph === 'mapper-stub';
  // The specifier of `src/<name>` as written in a file of directory `from` ('' or 'src').
  const sp = (name, from) => {
    if (pkg && name === 'm') return ext;
    const file = native && name === 'm' ? 'm.cjs' : name;
    return alias && name === 'm' ? '@/m' : from === 'src' ? `./${file}` : `./src/${file}`;
  };
  // The value of a mock of the target: only the names the target exports (an `export *`
  // of two sources with one name fails).
  const names = target === 'm' ? ['v'] : target === 'barrel' ? ['v', 'n'] : ['d'];
  const obj = (val) => `{ ${names.map((n) => `${n}: '${val}'`).join(', ')} }`;

  files['src/deep.ts'] = `${mark('src/deep.ts')}export const d = 'deep';\n`;
  files['src/m.ts'] = `import { d } from './deep';\n${mark('src/m.ts')}export type V = string;\nexport const v = 'real:' + d;\nexport const f = () => 'realf';\n`;
  const userFrom = spec.graph.startsWith('barrel') ? './barrel' : sp('m', 'src');
  if (native) {
    files['src/m.cjs'] = `${mark('src/m.cjs')}module.exports = { v: 'real' };\n`;
    files['src/user.ts'] =
      (jest ? '' : "import { createRequire } from 'node:module';\nconst require = createRequire(import.meta.url);\n") +
      `const m = require('./m.cjs');\n${mark('src/user.ts')}export const use = () => m.v;\n`;
  } else if (pkg) {
    files['src/user.ts'] = `import { v } from '${ext}';\n${mark('src/user.ts')}export const use = () => v;\n`;
  } else if (spec.graph === 'dynamic') {
    files['src/user.ts'] = `${mark('src/user.ts')}export const use = async () => (await import(${q(userFrom)})).v;\n`;
  } else if (spec.graph === 'unrequested') {
    // Nothing requests m: whatever is registered for it, neither it nor a mock of it loads.
    files['src/user.ts'] = `${mark('src/user.ts')}export const use = () => 'none';\n`;
  } else if (spec.graph === 'typeonly') {
    files['src/user.ts'] = `import type { V } from ${q(userFrom)};\n${mark('src/user.ts')}export const use = (): V => 'none';\n`;
  } else if (spec.graph === 'bound') {
    // The subject names m to a loader in another file, which loads what it is given: no
    // file has an import of m. Cross-file binding makes the load a dependency of the
    // subject (an edge of the derived layer, as the manual-mock edges are), and the mock
    // rule has to cut or keep that edge like a stored one. Without binding the loader's
    // open record keeps the test selected on every change.
    files['src/loader.ts'] = `${mark('src/loader.ts')}export const load = (p: string) => import(p);\n`;
    files['src/user.ts'] =
      `import { load } from './loader';\n${mark('src/user.ts')}export const use = async () => (await load(${q(userFrom)})).v;\n`;
  } else {
    files['src/user.ts'] = `import { v } from ${q(userFrom)};\n${mark('src/user.ts')}export const use = () => v;\n`;
  }
  if (spec.graph.startsWith('barrel')) {
    files['src/n.ts'] = `${mark('src/n.ts')}export const n = 'n';\n`;
    files['src/barrel.ts'] = `export * from './m';\nexport * from './n';\n${mark('src/barrel.ts')}`;
  }
  const imports = [];
  if (spec.graph === 'second' || spec.graph === 'barrel-other') {
    const dep = spec.graph === 'second' ? 'deep' : 'n';
    const name = spec.graph === 'second' ? 'd' : 'n';
    files['src/other.ts'] = `import { ${name} } from './${dep}';\n${mark('src/other.ts')}export const o = ${name};\n`;
    imports.push("import { o } from './src/other';");
  }
  if (spec.graph === 'testimports') imports.push(`import { v as tv } from ${q(sp('m', ''))};`);
  if (spec.graph === 'alias') cfg.alias = 'src';
  if (spec.graph === 'mapper-stub') {
    cfg.alias = 'stub';
    files['src/stub.ts'] = `${mark('src/stub.ts')}export const v = 'stub';\n`;
  }

  // The mock statements, as written in a file of directory `from`.
  const stmts = (from) => {
    const s = sp(target, from);
    const fake = from === 'src' ? './fake' : './src/fake';
    switch (spec.form) {
      case 'factory':
        return [A.mock(s, `() => (${obj('mock')})`)];
      case 'factory-ext':
        return [A.mock(pkg || s.endsWith('.cjs') ? s : `${s}.ts`, `() => (${obj('mock')})`)];
      case 'factory-import':
        return [`vi.mock(import(${q(s)}), () => (${obj('mock')}));`];
      case 'factory-var':
        // A `mock`-prefixed variable (jest) or a hoisted value (vitest), read lazily.
        return jest
          ? ["const mockV = 'mockvar';", A.mock(s, `() => ({ ${names.map((n) => `get ${n}() { return mockV; }`).join(', ')} })`)]
          : ["const hoisted = vi.hoisted(() => ({ v: 'mockvar' }));", A.mock(s, `() => ({ ${names.map((n) => `${n}: hoisted.v`).join(', ')} })`)];
      case 'factory-fake':
        files['src/fake.ts'] = `${mark('src/fake.ts')}${names.map((n) => `export const ${n} = 'fake';\n`).join('')}`;
        return [A.mock(s, jest ? `() => require(${q(fake)})` : `async () => await import(${q(fake)})`)];
      case 'partial':
        return [A.mock(s, jest ? `() => ({ ...jest.requireActual(${q(s)}), extra: 1 })` : 'async (importOriginal) => ({ ...(await importOriginal()), extra: 1 })')];
      case 'partial-actual':
        return [A.mock(s, jest ? `() => { const actual = jest.requireActual(${q(s)}); return { ...actual, extra: 1 }; }` : `async () => ({ ...(await vi.importActual(${q(s)})), extra: 1 })`)];
      case 'manual':
        files['src/mockdep.ts'] = `${mark('src/mockdep.ts')}export const md = 'mockdep';\n`;
        {
          // Next to a user module; in the root for a package.
          const [file, dep] = pkg ? ['__mocks__/extpkg.ts', '../src/mockdep'] : [`src/__mocks__/${native ? 'm.cjs' : `${target}.ts`}`, '../mockdep'];
          files[file] = native
            ? `${mark(file)}module.exports = { v: 'manual:' + require('../mockdep').md };\n`
            : `import { md } from ${q(dep)};\n${mark(file)}${names.map((n) => `export const ${n} = 'manual:' + md;\n`).join('')}`;
        }
        return [A.mock(s)];
      case 'auto':
        return [A.mock(s)];
      case 'spy':
        return [A.mock(s, '{ spy: true }')];
      case 'virtual':
        return [A.mock(s, `() => (${obj('virtual')})`, '{ virtual: true }'), A.mock('not-a-module', '() => ({})', '{ virtual: true }')];
      case 'domock':
        return [A.doMock(s, `() => (${obj('domock')})`)];
      case 'setmock':
        return [`jest.setMock(${q(s)}, ${obj('setmock')});`];
      default:
        return [];
    }
  };

  const top = [];
  // Statements of the test body before the subject is loaded (style `dynamic`) and after.
  const pre = [];
  const body = [];
  const helperLines = [];
  let helperFirst = false;
  let helperBySetup = false;
  switch (spec.where) {
    case 'top':
      top.push(...stmts(''));
      break;
    case 'setup':
    case 'setupAfterEnv':
      files['setup.ts'] = `${jest || spec.neg === 'config-globals' ? A.head : "import { vi } from 'vitest';\n"}${mark('setup.ts')}${stmts('').join('\n')}\n`;
      cfg[spec.where].push('setup.ts');
      break;
    case 'helper-before':
    case 'helper-after':
      helperLines.push(...stmts('src'));
      helperFirst = spec.where === 'helper-before';
      break;
    case 'setup-helper':
    case 'setupAfterEnv-helper':
      // A helper that a setup file imports: the setup closure.
      helperLines.push(...stmts('src'));
      helperBySetup = true;
      files['setup.ts'] = `${spec.neg === 'setup-helper-late' ? "import './src/user';\n" : ''}import './src/helper';\n${mark('setup.ts')}`;
      cfg[spec.where === 'setup-helper' ? 'setup' : 'setupAfterEnv'].push('setup.ts');
      break;
    case 'beforeAll':
      top.push(`beforeAll(() => {\n  ${stmts('').join('\n  ')}\n});`);
      break;
    case 'block':
      top.push(`if (true) {\n  ${stmts('').join('\n  ')}\n}`);
      break;
    case 'fn':
      top.push(`function install() {\n  ${stmts('').join('\n  ')}\n}\ninstall();`);
      break;
    case 'inTest':
      pre.push(...stmts(''));
      break;
  }

  const s = sp(target, '');
  const reload = `seen.push(await ${A.load('./src/user')}.use());`;
  switch (spec.neg) {
    case 'actual-in-test':
      body.push(`seen.push(${A.actual(s)}.v);`);
      break;
    case 'actual-in-helper':
      helperLines.push(`export const real = ${jest ? '' : 'async '}() => ${A.actual(sp(target, 'src'))};`);
      body.push(`seen.push((await ${A.load('./src/helper')}.real()).v);`);
      break;
    case 'unmock-top':
      top.push(A.unmock(s));
      break;
    case 'unmock-reset':
      body.push(A.unmock(s), A.reset(), reload);
      break;
    case 'dontmock-reset':
      body.push(A.dontMock(s), A.reset(), reload);
      break;
    case 'reset-reload':
      body.push(A.reset(), reload);
      break;
    case 'isolate':
      body.push("jest.isolateModules(() => {\n      seen.push(require('./src/user').use());\n    });");
      break;
    case 'domock-override':
      body.push(A.doMock(s, `() => (${obj('late')})`), A.reset(), reload);
      break;
    case 'domock-bare':
      body.push(A.doMock(s), A.reset(), reload);
      break;
    case 'load-direct':
      body.push(`seen.push(${A.load(s)}.v);`);
      break;
    case 'actual-of-user':
      body.push(`seen.push(await ${A.actual('./src/user')}.use());`);
      break;
    case 'automock-user':
      // The runner loads the real subject to build its automock; what the subject imports
      // still goes through the registry.
      top.push(A.mock('./src/user'));
      break;
    case 'config-globals':
      cfg.vextra.push('globals: true');
      break;
    case 'config-noisolate':
      // A second test file loads the real module in the same worker.
      cfg.vextra.push('isolate: false', 'fileParallelism: false');
      cfg.include = '*.test.ts';
      files['a.test.ts'] = `${A.head}import { use } from './src/user';\ntest('a', () => {\n  use();\n});\n`;
      break;
    case 'helper-bare-mock':
      helperLines.push(A.mock(sp(target, 'src')));
      break;
    case 'config-resetModules':
      cfg.extra.push('resetModules: true');
      break;
    case 'config-automock':
      cfg.extra.push('automock: true');
      break;
    case 'setup-loads-m':
      files['setup0.ts'] = `import ${q(sp('m', ''))};\n${mark('setup0.ts')}`;
      cfg.setup.unshift('setup0.ts');
      break;
    case 'setup-loads-user':
      files['setup0.ts'] = `import './src/user';\n${mark('setup0.ts')}`;
      cfg.setup.unshift('setup0.ts');
      break;
    case 'setup-unmock':
      files['setup0.ts'] = `${jest ? A.head : "import { vi } from 'vitest';\n"}${mark('setup0.ts')}${A.unmock(s)}\n`;
      cfg.setupAfterEnv.push('setup0.ts');
      break;
    case 'helper-bare-manual':
      // A helper registers the manual mock of the target, whatever the test does with it
      // (a factory of the test stays in jest; in vitest the later call wins).
      files['src/mockdep.ts'] ??= `${mark('src/mockdep.ts')}export const md = 'mockdep';\n`;
      files[`src/__mocks__/${target}.ts`] ??= `import { md } from '../mockdep';\n${mark(`src/__mocks__/${target}.ts`)}${names.map((n) => `export const ${n} = 'manual:' + md;\n`).join('')}`;
      helperLines.push(A.mock(sp(target, 'src')));
      break;
    case 'helper-loads-m':
      // The helper requests m itself (after its own mock calls, which are hoisted).
      helperLines.push(`import * as hm from ${q(sp('m', 'src'))};\nexport const hv = Object.keys(hm).length;`);
      break;
  }
  if (helperLines.length) {
    files['src/helper.ts'] = `${jest ? A.head : "import { vi } from 'vitest';\n"}${mark('src/helper.ts')}${helperLines.join('\n')}\nexport const h = 1;\n`;
    if (!helperBySetup && (spec.neg !== 'actual-in-helper' || spec.where.startsWith('helper'))) {
      (helperFirst ? imports.unshift.bind(imports) : imports.push.bind(imports))("import './src/helper';");
    }
  }

  const subject =
    spec.style === 'esm' ? "import { use } from './src/user';" : spec.style === 'cjs' ? "const { use } = require('./src/user');" : '';
  const head = helperFirst ? [...imports, subject] : [subject, ...imports];
  const obs = `${spec.runner}__${spec.name}.json`;
  files['t.test.ts'] =
    `${A.head}${jest && !R.esm ? "const fs = require('fs');\n" : "import fs from 'node:fs';\n"}` +
    `${head.filter(Boolean).join('\n')}\n${mark('t.test.ts')}${top.join('\n')}\n` +
    `test('obs', async () => {\n  const seen = [];\n  try {\n` +
    (spec.style === 'dynamic' ? `    ${pre.join('\n    ')}\n    const { use } = ${A.load('./src/user')};\n` : '') +
    `    seen.push(await use());\n` +
    (spec.graph === 'testimports' ? '    seen.push(tv);\n' : '') +
    (imports.some((i) => i.includes('other')) ? '    seen.push(o);\n' : '') +
    `    ${body.join('\n    ')}\n` +
    `  } catch (e) {\n    seen.push('ERR ' + String(e && e.message).split('\\n')[0]);\n  }\n` +
    `  fs.writeFileSync(process.env.MOCK_ORACLE_OBS + '/${obs}', JSON.stringify({ loaded: globalThis.__loaded || [], seen }));\n});\n`;

  const setup = [...cfg.setup, ...(jest ? [] : cfg.setupAfterEnv)];
  if (jest) {
    const mapper =
      cfg.alias === 'src' ? "moduleNameMapper: { '^@/(.*)$': '<rootDir>/src/$1' }," : cfg.alias === 'stub' ? "moduleNameMapper: { '^@/m$': '<rootDir>/src/stub.ts' }," : '';
    files['jest.config.js'] =
      (R.next ? "const nextJest = require('next/jest');\nconst custom = {\n" : 'module.exports = {\n') +
      `  displayName: '${spec.name}',\n  testEnvironment: 'node',\n  testMatch: ['<rootDir>/t.test.ts'],\n` +
      (R.next ? '' : `  transform: { '\\\\.[jt]sx?$': ${R.transform} },\n`) +
      (R.esm ? "  extensionsToTreatAsEsm: ['.ts'],\n" : '') +
      (setup.length ? `  setupFiles: [${setup.map((f) => `'<rootDir>/${f}'`).join(', ')}],\n` : '') +
      (cfg.setupAfterEnv.length ? `  setupFilesAfterEnv: [${cfg.setupAfterEnv.map((f) => `'<rootDir>/${f}'`).join(', ')}],\n` : '') +
      (mapper ? `  ${mapper}\n` : '') +
      cfg.extra.map((e) => `  ${e},\n`).join('') +
      `};\n` +
      (R.next ? NEXT_CONFIG : '');
  } else {
    const aliasLine =
      cfg.alias === 'src'
        ? "  resolve: { alias: { '@': path.join(here, 'src') } },\n"
        : cfg.alias === 'stub'
          ? "  resolve: { alias: { '@/m': path.join(here, 'src/stub.ts') } },\n"
          : '';
    files['vitest.config.mjs'] =
      `import path from 'node:path';\nimport { fileURLToPath } from 'node:url';\nimport { defineConfig } from 'vitest/config';\n` +
      `const here = path.dirname(fileURLToPath(import.meta.url));\n` +
      `export default defineConfig({\n${aliasLine}  test: {\n    name: '${spec.name}',\n    include: ['${cfg.include}'],\n` +
      cfg.vextra.map((e) => `    ${e},\n`).join('') +
      (setup.length ? `    setupFiles: [${setup.map((f) => `'./${f}'`).join(', ')}],\n` : '') +
      `  },\n});\n`;
  }
  return { files, deps: missing ? { 'missing-pkg': '1.0.0' } : {} };
}

// ---- the cases: every value of every dimension once, then `--gen` random combinations ----

function fixed() {
  const out = [];
  const base = { graph: 'direct', form: 'factory', where: 'top', neg: 'none', style: 'esm' };
  for (const runner of RUNNERS) {
    const add = (o) => out.push({ runner, ...base, ...o });
    for (const form of Object.keys(FORMS)) add({ form });
    for (const graph of GRAPHS.slice(1)) add({ graph });
    for (const where of WHERES.slice(1)) add({ where, style: where === 'inTest' ? 'dynamic' : 'esm' });
    for (const neg of Object.keys(NEGS).slice(1)) add({ neg });
    for (const style of Object.keys(STYLES).slice(1)) add({ style });
    // Pairs that matter on their own.
    add({ form: 'manual', where: 'setup' });
    add({ form: 'manual', neg: 'actual-in-test' });
    add({ form: 'manual', graph: 'barrel-whole' });
    add({ form: 'auto', where: 'setup' });
    add({ form: 'factory', where: 'setup', neg: 'setup-loads-m' });
    add({ form: 'factory', where: 'setup', graph: 'alias' });
    add({ form: 'factory', style: 'cjs', neg: 'unmock-top' });
    add({ form: 'factory', graph: 'dynamic', neg: 'reset-reload' });
    add({ form: 'factory', graph: 'barrel-whole', neg: 'actual-in-helper' });
    add({ form: 'factory-fake', graph: 'second' });
    add({ form: 'none', neg: 'helper-bare-mock' });
    add({ form: 'manual', graph: 'pkg' });
    add({ form: 'factory', where: 'setup', neg: 'config-globals' });
    add({ form: 'manual', neg: 'config-globals' });
    add({ form: RUNNER[runner].kind === 'jest' ? 'virtual' : 'factory', graph: 'pkg-missing' });
    add({ form: RUNNER[runner].kind === 'jest' ? 'virtual' : 'factory', graph: 'pkg-missing', where: 'setup' });
    add({ form: 'manual', graph: 'native-require' });
    add({ form: 'factory', where: 'setup', neg: 'actual-in-helper' });
    add({ form: 'factory', where: 'setup', neg: 'domock-bare' });
    add({ form: 'manual', where: 'setup', neg: 'unmock-reset' });
    add({ form: 'factory', where: 'setup', style: 'dynamic' });
    // The module is loaded through a site that cross-file binding binds to the subject:
    // the request is an edge of the derived layer. Not replaced at all (the bound edge
    // alone must select), replaced, replaced and loaded for real, and with the manual
    // mock that a helper registers (two derived layers on one path: subject → m → mock).
    add({ graph: 'bound', form: 'none' });
    add({ graph: 'bound', form: 'manual' });
    add({ graph: 'bound', form: 'auto' });
    add({ graph: 'bound', form: 'partial-actual' });
    add({ graph: 'bound', neg: 'actual-in-test' });
    add({ graph: 'bound', neg: 'unmock-top' });
    add({ graph: 'bound', where: 'setup' });
    add({ graph: 'bound', form: 'manual', where: 'helper-before' });
    add({ graph: 'bound', form: 'manual', where: 'setup-helper' });
    add({ graph: 'bound', form: 'manual', neg: 'helper-bare-manual' });
    add({ graph: 'bound', style: 'dynamic', neg: 'reset-reload' });
    // A mock without a factory loads nothing until the module is requested.
    for (const form of ['manual', 'auto']) {
      for (const where of ['top', 'setup', 'helper-before', 'setup-helper']) {
        add({ graph: 'unrequested', form, where });
        if (where.includes('helper')) add({ form, where, neg: 'helper-loads-m' });
      }
      add({ graph: 'typeonly', form, where: 'helper-before' });
      add({ graph: 'unrequested', form, where: 'helper-before', neg: 'helper-loads-m' });
      add({ graph: 'dynamic', form, where: 'helper-before' });
      add({ graph: 'alias', form, where: 'setup-helper' });
    }
    // The setup closure: a helper a setup file imports, before and after the subject.
    for (const form of ['factory', 'manual', 'auto', 'partial']) {
      add({ form, where: 'setup-helper' });
      add({ form, where: 'setup-helper', neg: 'setup-helper-late' });
      add({ form, where: 'setup-helper', neg: 'setup-loads-m' });
      if (RUNNER[runner].kind === 'jest') add({ form, where: 'setupAfterEnv-helper' });
    }
    add({ form: 'factory', where: 'setup-helper', graph: 'second' });
    add({ form: 'factory', where: 'setup-helper', neg: 'actual-in-test' });
    add({ form: 'factory', where: 'setup-helper', neg: 'reset-reload' });
    // A helper registers the manual mock of a module the test replaces or just imports.
    for (const form of ['factory', 'none', 'manual', 'auto']) {
      add({ form, neg: 'helper-bare-manual' });
      add({ form, neg: 'helper-bare-manual', style: 'dynamic' });
      add({ form, neg: 'helper-bare-manual', graph: 'unrequested' });
    }
    add({ form: 'factory', where: 'setup', neg: 'helper-bare-manual', style: 'dynamic' });
    add({ form: 'factory', where: 'helper-before', neg: 'helper-bare-manual' });
  }
  return out;
}

function mulberry32(a) {
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function generated(n, seed) {
  const rnd = mulberry32(seed);
  const pick = (xs) => xs[Math.floor(rnd() * xs.length)];
  const out = [];
  let guard = 0;
  while (out.length < n && guard++ < n * 200) {
    const spec = {
      runner: pick(RUNNERS),
      graph: pick(GRAPHS),
      form: pick(Object.keys(FORMS)),
      where: pick(WHERES),
      neg: pick(Object.keys(NEGS)),
      style: pick(Object.keys(STYLES)),
    };
    out.push(spec);
  }
  return out;
}

const shape = (s) => `${s.graph} ${s.form} ${s.where} ${s.neg} ${s.style}`;
const seenShapes = new Set();
const cases = [];
for (const spec of [...fixed(), ...generated(GEN, SEED)]) {
  if (!RUNNERS.includes(spec.runner) || !valid(spec)) continue;
  const key = `${spec.runner} ${shape(spec)}`;
  if (seenShapes.has(key) || (ONLY && !key.includes(ONLY))) continue;
  seenShapes.add(key);
  spec.name = `c${cases.length}`;
  cases.push({ spec, key, dir: `${spec.runner}/cases/${spec.name}`, ...build(spec) });
}
if (!cases.length) {
  console.error('mock-oracle: no case matches');
  process.exit(2);
}

// ---- the work directory ----

const write = (rel, text) => {
  const p = path.join(WORK, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, text);
};
fs.mkdirSync(WORK, { recursive: true });
for (const r of Object.keys(RUNNER)) fs.rmSync(path.join(WORK, r), { recursive: true, force: true });
const OBS = path.join(WORK, 'obs');
const PKG = {
  name: 'mock-oracle',
  private: true,
  version: '1.0.0',
  devDependencies: {
    jest: '29.7.0',
    'babel-jest': '29.7.0',
    '@babel/core': '7.26.0',
    '@babel/preset-env': '7.26.0',
    '@babel/preset-typescript': '7.26.0',
    '@swc/core': '1.10.1',
    '@swc/jest': '0.2.37',
    'ts-jest': '29.2.5',
    typescript: '5.7.2',
    vitest: '3.2.4',
    next: '15.1.3',
    react: '19.0.0',
    'react-dom': '19.0.0',
  },
};
const pkgText = JSON.stringify(PKG, null, 1);
if (!fs.existsSync(path.join(WORK, 'node_modules/.bin/vitest')) || fs.readFileSync(path.join(WORK, 'package.json'), 'utf8') !== pkgText) {
  write('package.json', pkgText);
  console.error('mock-oracle: npm install ...');
  execFileSync('npm', ['install', '--no-audit', '--no-fund', '--silent'], { cwd: WORK, stdio: ['ignore', 'ignore', 'inherit'] });
}
write('.gitignore', 'node_modules\nobs\n.cache\n');
// A third-party package for the mocks of packages.
write('node_modules/extpkg/package.json', '{"name":"extpkg","version":"1.0.0","main":"index.js"}');
write('node_modules/extpkg/index.js', "exports.v = 'ext';\n");
for (const c of cases) {
  write(`${c.dir}/package.json`, JSON.stringify({ name: `${c.spec.runner}-${c.spec.name}`, version: '1.0.0', private: true, dependencies: c.deps }));
  for (const [rel, text] of Object.entries(c.files)) write(`${c.dir}/${rel}`, text);
  // The second test file of a shared registry is another entry, not a file of this one.
  c.sources = Object.keys(c.files)
    .filter((f) => (f.endsWith('.ts') || f.endsWith('.cjs')) && f !== 'a.test.ts')
    .sort();
}
for (const r of RUNNERS) {
  if (!cases.some((c) => c.spec.runner === r)) continue;
  if (RUNNER[r].wrapper) write(`${r}/${{ jw: 'wrap', jx: 'own', jn: 'nextwrap' }[r]}.cjs`, RUNNER[r].wrapper);
  if (RUNNER[r].kind === 'jest') write(`${r}/jest.config.js`, "module.exports = { projects: ['<rootDir>/cases/*'] };\n");
  else write(`${r}/vitest.config.mjs`, "import { defineConfig } from 'vitest/config';\nexport default defineConfig({ test: { projects: ['cases/*/vitest.config.mjs'] } });\n");
}

// ---- run the tests ----

// Runs every case of the selected runners (or one case) and returns the observations by
// case name.
function runAll(only) {
  fs.rmSync(OBS, { recursive: true, force: true });
  fs.mkdirSync(OBS, { recursive: true });
  for (const r of RUNNERS) {
    const env = { ...process.env, MOCK_ORACLE_OBS: OBS, CI: '1', NODE_OPTIONS: RUNNER[r].esm ? '--experimental-vm-modules' : '' };
    if (!cases.some((c) => c.spec.runner === r) || (only && only.spec.runner !== r)) continue;
    const cwd = path.join(WORK, r);
    const cmd =
      RUNNER[r].kind === 'jest'
        ? ['jest', '--config', 'jest.config.js', '--cacheDirectory', path.join(WORK, '.cache', r), ...(only ? ['--selectProjects', only.spec.name] : [])]
        : ['vitest', 'run', '--config', 'vitest.config.mjs', ...(only ? ['--project', only.spec.name] : [])];
    const res = spawnSync(path.join(WORK, 'node_modules/.bin', cmd[0]), cmd.slice(1), { cwd, env, encoding: 'utf8', timeout: 900000, maxBuffer: 64 << 20 });
    fs.writeFileSync(path.join(WORK, `${r}.log`), `${res.stdout}\n${res.stderr}`);
    if (VERBOSE) console.error(`[${r}] exit ${res.status}`);
  }
  const out = {};
  for (const c of cases) {
    try {
      out[c.key] = JSON.parse(fs.readFileSync(path.join(OBS, `${c.spec.runner}__${c.spec.name}.json`), 'utf8'));
    } catch {}
  }
  return out;
}

const bump = (c, files, to) => {
  for (const f of files) {
    const p = path.join(WORK, c.dir, f);
    fs.writeFileSync(p, fs.readFileSync(p, 'utf8').replace(/@\d+'\);/, `@${to}');`));
  }
};
const ran = (o, v) => new Set((o?.loaded ?? []).filter((x) => x.endsWith(`@${v}`)).map((x) => x.slice(0, -2)));

const t0 = Date.now();
const baseObs = runAll();
for (const c of cases) bump(c, c.sources, 2);
const headObs = runAll();
for (const c of cases) bump(c, c.sources, 1);
const total = { cases: cases.length, crashed: 0, files: 0, ran: 0, unstable: 0, selected: 0, open: 0, freed: 0, kept: 0, manifests: 0, findings: 0, graphFindings: 0 };
for (const c of cases) {
  c.base = baseObs[c.key];
  c.head = headObs[c.key];
  if (!c.base || !c.head) {
    c.crashed = true;
    total.crashed++;
    continue;
  }
  c.ran = ran(c.base, 1);
  // The head run bumped every file: the files that ran are the files whose bump shows.
  const again = ran(c.head, 2);
  c.unstable = c.ran.size !== again.size || [...c.ran].some((f) => !again.has(f)) || JSON.stringify(c.base.seen) !== JSON.stringify(c.head.seen);
  if (c.unstable) total.unstable++;
}
console.error(`mock-oracle: ${cases.length} cases ran twice in ${((Date.now() - t0) / 1000).toFixed(0)} s`);

// `--single N`: N single-file mutations run alone; the observation must change exactly
// when the joint head run says the file ran.
let singles = 0;
let singleMismatch = 0;
if (SINGLE > 0) {
  const rnd = mulberry32(SEED + 7);
  const live = cases.filter((c) => !c.crashed);
  for (let i = 0; i < SINGLE && live.length; i++) {
    const c = live[Math.floor(rnd() * live.length)];
    const f = c.sources[Math.floor(rnd() * c.sources.length)];
    bump(c, [f], 2);
    const o = runAll(c)[c.key];
    bump(c, [f], 1);
    singles++;
    const changed = !o || JSON.stringify(o) !== JSON.stringify(c.base);
    if (changed !== c.ran.has(f)) {
      singleMismatch++;
      console.log(`SINGLE MISMATCH ${c.key}: ${f} ran=${c.ran.has(f)} observation changed=${changed}`);
    }
  }
}

// ---- what the trace selects ----

execFileSync(BIN, ['graph', 'build', '--root', WORK], { cwd: WORK, stdio: ['ignore', 'ignore', 'inherit'] });
// One call per slot: slot k changes the k-th file of every case (cases share nothing, so
// a test can only be selected by the file of its own case).
function selections(rule) {
  const env = { ...process.env, TURBO_TRACE_DEBUG_MOCKS: rule ? '1' : '0' };
  const sel = new Map();
  const slots = Math.max(...cases.map((c) => c.sources.length));
  for (let k = 0; k < slots; k++) {
    const changed = cases.filter((c) => !c.crashed && c.sources[k]).map((c) => `${c.dir}/${c.sources[k]}`);
    if (!changed.length) continue;
    const out = execFileSync(BIN, ['affected', '--tests', 'jest', '--tests', 'vitest', '--changed', ...changed, '--root', WORK, '--no-refresh', '--format', 'json'], {
      cwd: WORK,
      env,
      encoding: 'utf8',
      maxBuffer: 256 << 20,
    });
    for (const r of JSON.parse(out).results) sel.set(`${k} ${r.entry}`, r.cause ? 'change' : 'open');
  }
  // The manifests: a mock of a package does not make a test independent of the manifest
  // that declares it.
  const manifests = cases.filter((c) => !c.crashed).map((c) => `${c.dir}/package.json`);
  const out = execFileSync(BIN, ['affected', '--tests', 'jest', '--tests', 'vitest', '--changed', ...manifests, '--root', WORK, '--no-refresh', '--format', 'json'], {
    cwd: WORK,
    env,
    encoding: 'utf8',
    maxBuffer: 256 << 20,
  });
  for (const r of JSON.parse(out).results) sel.set(`manifest ${r.entry}`, r.cause ? 'change' : 'open');
  return sel;
}
const off = selections(false);
const on = selections(true);

const findings = [];
const byShape = [];
for (const c of cases) {
  if (c.crashed) continue;
  const entry = `${c.dir}/t.test.ts`;
  const row = { case: c.key, ran: [...c.ran].sort(), seen: c.base.seen, freed: [], kept: [], findings: [] };
  c.sources.forEach((f, k) => {
    total.files++;
    const a = off.get(`${k} ${entry}`);
    const b = on.get(`${k} ${entry}`);
    if (b === 'open') total.open++;
    if (b) total.selected++;
    if (c.ran.has(f)) {
      total.ran++;
      if (!b) {
        const kind = a ? 'rule' : 'graph';
        if (kind === 'rule') total.findings++;
        else total.graphFindings++;
        row.findings.push(f);
        findings.push({ case: c.key, file: f, kind, dir: c.dir });
      }
    } else if (a && !b) {
      total.freed++;
      row.freed.push(f);
    } else if (b) {
      total.kept++;
      row.kept.push(f);
    }
  });
  total.manifests += off.has(`manifest ${entry}`) ? 1 : 0;
  if (off.has(`manifest ${entry}`) && !on.has(`manifest ${entry}`)) {
    total.findings++;
    findings.push({ case: c.key, file: 'package.json (selected without the rule)', kind: 'rule', dir: c.dir });
  }
  byShape.push(row);
}

for (const f of findings) console.log(`FINDING (${f.kind}) ${f.case}: ${f.file} runs in the test; a change to it does not select the test [${f.dir}]`);
for (const c of cases) {
  if (c.crashed) console.log(`CRASHED ${c.key} [${c.dir}]: no observation (see ${c.spec.runner}.log)`);
  else if (c.unstable) console.log(`UNSTABLE ${c.key} [${c.dir}]: base and head runs differ in what ran`);
}
if (VERBOSE) {
  for (const r of byShape) {
    console.log(`${r.case}\n    ran: ${r.ran.join(' ')}\n    seen: ${JSON.stringify(r.seen)}\n    freed: ${r.freed.join(' ') || '-'}   kept though not run: ${r.kept.join(' ') || '-'}`);
  }
}
console.log(
  `mock-oracle: ${total.cases} cases (${total.crashed} crashed, ${total.unstable} unstable), ${total.files} (test, file) pairs: ${total.ran} ran; ` +
    `not run: ${total.freed} freed by the rule, ${total.kept} still selected; ${total.open} selected through open uncertainty; ` +
    `${total.manifests} tests selected by their manifest; ` +
    `${total.findings} FINDINGS of the rule, ${total.graphFindings} of the graph` +
    (SINGLE ? `; ${singles} single mutations, ${singleMismatch} mismatches` : ''),
);
const jsonOut = opt('--json', '');
if (jsonOut) {
  fs.mkdirSync(path.dirname(path.resolve(jsonOut)), { recursive: true });
  fs.writeFileSync(jsonOut, JSON.stringify({ total, findings, cases: byShape }, null, 1));
}
if (!flag('--keep')) for (const r of Object.keys(RUNNER)) fs.rmSync(path.join(WORK, r), { recursive: true, force: true });
process.exit(total.findings > 0 || total.graphFindings > 0 || singleMismatch > 0 ? 1 : 0);
