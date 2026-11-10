// Preload of the cross-file binding oracle (see oracle.mjs): records every project file the
// process loads, through `require`, `import()` or a static import, and writes the list to
// $XFILE_ORACLE_OUT at exit (also after a crash).
//
// It also makes a test file runnable as a plain Node script the way a test runner with a
// transpiler would run it:
//   - `describe` / `it` / `test` / the hooks call their callback at once, `expect` accepts
//     anything; an error inside a callback is recorded, the rest of the file still runs;
//   - a `.js` file with `import` / `export` declarations is loaded as an ES module that also
//     has `require`, `__filename` and `__dirname` (what the CommonJS output of a transpiler
//     has), and relative imports resolve without an extension and to a directory's index
//     file, as a bundler-style resolver does;
//   - a specifier that matches a pattern of $XFILE_ORACLE_MAPPER (the `moduleNameMapper` of
//     the package's jest configuration, `<rootDir>` filled in) is the file the pattern maps
//     it to.
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const Module = require('node:module');
const { fileURLToPath, pathToFileURL } = require('node:url');

const ROOT = fs.realpathSync(process.env.XFILE_ORACLE_ROOT);
const OUT = process.env.XFILE_ORACLE_OUT;
const loaded = new Set();
const errors = [];
const writeOut = fs.writeFileSync.bind(fs);
const readText = fs.readFileSync.bind(fs);
const realpath = fs.realpathSync.bind(fs);
const stat = fs.statSync.bind(fs);
const NM = `${path.sep}node_modules${path.sep}`;

// The repository-relative path of a project file (symlinks resolved: a workspace package
// loaded through node_modules is the package's own file), or null.
function projectFile(file) {
  let p = file;
  try {
    p = realpath(file);
  } catch {}
  if (!p.startsWith(ROOT + path.sep) || p.includes(NM)) return null;
  return path.relative(ROOT, p).split(path.sep).join('/');
}

function note(file) {
  const rel = projectFile(file);
  if (rel) loaded.add(rel);
}

const isFile = (p) => {
  try {
    return stat(p).isFile();
  } catch {
    return false;
  }
};

// What a file with ES module syntax gets in addition (on the first line: line numbers of
// the program stay the same).
const PRELUDE =
  "import { createRequire as __xo_createRequire } from 'node:module'; const require = __xo_createRequire(import.meta.url); " +
  'const __filename = import.meta.filename; const __dirname = import.meta.dirname; ';
const ESM_SYNTAX = /^[ \t]*(import\s*[{*'"]|import\s+\w|export\s*[{*]|export\s+\w)/m;
const EXTENSIONS = ['.js', '.cjs', '.mjs', '.json', '/index.js', '/index.cjs', '/index.mjs'];
const MAPPER = JSON.parse(process.env.XFILE_ORACLE_MAPPER || '[]').map(([pattern, target]) => [new RegExp(pattern), target]);

// The file a mapped specifier names, or null.
function mapped(specifier) {
  for (const [pattern, target] of MAPPER) {
    const m = pattern.exec(specifier);
    if (!m) continue;
    const base = target.replace(/\$(\d)/g, (_, i) => m[Number(i)] ?? '');
    for (const ext of ['', ...EXTENSIONS]) {
      if (isFile(base + ext)) return base + ext;
    }
  }
  return null;
}

Module.registerHooks({
  resolve(specifier, context, next) {
    const file = typeof specifier === 'string' && MAPPER.length ? mapped(specifier) : null;
    if (file) return next(file, context);
    try {
      return next(specifier, context);
    } catch (e) {
      const parent = context.parentURL && context.parentURL.startsWith('file:') ? fileURLToPath(context.parentURL) : null;
      if (typeof specifier !== 'string' || !parent || !/^(\.\.?(\/|$)|\/)/.test(specifier)) throw e;
      const base = path.resolve(path.dirname(parent), specifier.replace(/[?#].*$/s, ''));
      for (const ext of EXTENSIONS) {
        if (isFile(base + ext)) return next(pathToFileURL(base + ext).href, context);
      }
      throw e;
    }
  },
  load(url, context, next) {
    if (url.startsWith('file:')) {
      const file = fileURLToPath(url);
      note(file);
      if (projectFile(file) && /\.m?js$/.test(file)) {
        const text = readText(file, 'utf8');
        if (file.endsWith('.mjs') || ESM_SYNTAX.test(text)) return { format: 'module', source: PRELUDE + text, shortCircuit: true };
      }
    }
    return next(url, context);
  },
});

// The CommonJS loader, whatever the hooks saw: every file `require` resolved.
const origLoad = Module._load;
Module._load = function (request, parent, ...rest) {
  try {
    const file = Module._resolveFilename(request, parent);
    if (path.isAbsolute(file)) note(file);
  } catch {}
  return origLoad.call(this, request, parent, ...rest);
};

function fail(e) {
  errors.push(String(e && e.message ? e.message : e).split('\n')[0]);
}

function call(fn, ...args) {
  if (typeof fn !== 'function') return;
  try {
    const r = fn(...args);
    if (r && typeof r.then === 'function') r.then(undefined, fail);
  } catch (e) {
    fail(e);
  }
}

const each = (table) => (name, fn) => table.forEach((row) => (Array.isArray(row) ? call(fn, ...row) : call(fn, row)));
const block = Object.assign((name, fn) => call(fn), { each, only: (name, fn) => call(fn), skip: () => {} });
globalThis.describe = block;
globalThis.it = block;
globalThis.test = block;
for (const hook of ['beforeAll', 'beforeEach', 'afterAll', 'afterEach']) globalThis[hook] = (fn) => call(fn);
const matcher = new Proxy(function () {}, { get: (_, k) => (k === 'then' ? undefined : matcher), apply: () => matcher });
globalThis.expect = () => matcher;

process.on('exit', () => {
  for (const file of Object.keys(require.cache)) note(file);
  try {
    writeOut(OUT, JSON.stringify({ loaded: [...loaded].sort(), errors }));
  } catch {}
});
process.on('uncaughtException', (e) => {
  fail(e);
  process.exitCode = 0;
});
process.on('unhandledRejection', fail);
