// Preload of the value-flow oracle (see oracle.mjs): records the first argument of every
// tracked file API, `require` and `import()` call made by the program under test, with the
// line of the call in that program, and writes the list to $FLOW_ORACLE_OUT at exit.
//
// The program never touches the real file system outside its temporary directory: reads of
// missing files return stub data, writes outside $TMPDIR are dropped, and modules that do
// not resolve load as a stub.
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const Module = require('node:module');
const { fileURLToPath } = require('node:url');

const FILE = process.env.FLOW_ORACLE_FILE;
const OUT = process.env.FLOW_ORACLE_OUT;
const TMP = fs.realpathSync(require('node:os').tmpdir());
const seen = [];
const writeOut = fs.writeFileSync.bind(fs);

// The line of the call in the program under test: the first frame outside this file must
// be in the program (a file read made by the module loader while the program calls
// `require` is not a site). `anywhere`: the innermost program frame, whatever is between.
// `viaRequire`: the call came through a module's own `require` function (or its `resolve`),
// which is a frame of Node between the program and `Module.prototype.require`.
function line(anywhere = false, viaRequire = false) {
  const prev = Error.stackTraceLimit;
  Error.stackTraceLimit = 40;
  const stack = new Error().stack.split('\n').slice(1);
  Error.stackTraceLimit = prev;
  for (const f of stack) {
    if (f.includes(__filename)) continue;
    if (viaRequire && /^\s*at (require|\S*resolve) \(node:internal\/modules\/helpers:/.test(f)) continue;
    const at = f.indexOf(FILE);
    if (at < 0) {
      if (anywhere) continue;
      return 0;
    }
    const m = /^:(\d+):\d+/.exec(f.slice(at + FILE.length));
    if (m) return Number(m[1]);
  }
  return 0;
}

function show(v) {
  if (typeof v === 'string') return { t: 'string', v };
  if (v instanceof URL) return v.protocol === 'file:' ? { t: 'string', v: fileURLToPath(v) } : { t: 'url', v: v.href };
  if (Buffer.isBuffer(v)) return { t: 'string', v: v.toString() };
  return { t: v === null ? 'null' : typeof v, v: String(v) };
}

function record(api, kind, arg) {
  const l = line(false, kind === 'require');
  if (l) seen.push({ api, kind, line: l, ...show(arg) });
}

function inTmp(p) {
  try {
    const abs = path.resolve(String(p instanceof URL ? fileURLToPath(p) : p));
    return abs === TMP || abs.startsWith(TMP + path.sep) || abs.startsWith(require('node:os').tmpdir() + path.sep);
  } catch {
    return false;
  }
}

const stats = () => ({
  isFile: () => true,
  isDirectory: () => false,
  isSymbolicLink: () => false,
  size: 0,
  mtimeMs: 0,
  mtime: new Date(0),
});
// What a call returns when the real one fails (a missing fixture).
const STUB = {
  readFile: () => '{}',
  readJson: () => ({}),
  readdir: () => ['a.json', 'b.json'],
  opendir: () => ({ readSync: () => null, closeSync() {}, close() {}, read: async () => null, async *[Symbol.asyncIterator]() {} }),
  open: () => 99,
  exists: () => true,
  stat: stats,
  lstat: stats,
  access: () => undefined,
  createReadStream: () => ({ on() { return this; }, pipe() {}, close() {}, destroy() {} }),
};
const READS = {
  fs_read: ['readFile', 'createReadStream', 'open', 'exists', 'stat', 'lstat', 'access'],
  fs_read_dir: ['readdir', 'opendir'],
};
const WRITES = ['writeFile', 'appendFile', 'mkdir', 'rm', 'rmdir', 'unlink', 'copyFile', 'cp', 'rename', 'symlink', 'chmod'];

function patchSync(obj, name, kind, stub) {
  const orig = obj[name];
  if (typeof orig !== 'function') return;
  obj[name] = function (p, ...rest) {
    record(name, kind, p);
    try {
      return orig.call(this, p, ...rest);
    } catch {
      return stub();
    }
  };
}

function patchCallback(obj, name, kind, stub) {
  const orig = obj[name];
  if (typeof orig !== 'function') return;
  obj[name] = function (p, ...rest) {
    record(name, kind, p);
    const cb = typeof rest[rest.length - 1] === 'function' ? rest.pop() : null;
    if (!cb) return undefined;
    if (name === 'exists') {
      cb(true);
      return undefined;
    }
    try {
      orig.call(this, p, ...rest, (err, v) => (err ? cb(null, stub()) : cb(null, v)));
    } catch {
      cb(null, stub());
    }
    return undefined;
  };
}

function patchPromise(obj, name, kind, stub) {
  const orig = obj[name];
  if (typeof orig !== 'function') return;
  obj[name] = async function (p, ...rest) {
    record(name, kind, p);
    try {
      return await orig.call(this, p, ...rest);
    } catch {
      return name === 'open' ? { readFile: async () => '{}', close: async () => {} } : stub();
    }
  };
}

for (const [kind, names] of Object.entries(READS)) {
  for (const n of names) {
    patchSync(fs, `${n}Sync`, kind, STUB[n]);
    if (n === 'createReadStream') patchSync(fs, n, kind, STUB[n]);
    else patchCallback(fs, n, kind, STUB[n]);
    patchPromise(fs.promises, n, kind, STUB[n]);
  }
}
for (const n of WRITES) {
  for (const [obj, name, wrap] of [
    [fs, `${n}Sync`, (o) => function (p, ...r) { return inTmp(p) ? o.call(this, p, ...r) : undefined; }],
    [fs, n, (o) => function (p, ...r) { const cb = r[r.length - 1]; return inTmp(p) ? o.call(this, p, ...r) : typeof cb === 'function' ? cb(null) : undefined; }],
    [fs.promises, n, (o) => async function (p, ...r) { return inTmp(p) ? o.call(this, p, ...r) : undefined; }],
  ]) {
    if (typeof obj[name] === 'function') obj[name] = wrap(obj[name]);
  }
}
Module.syncBuiltinESMExports();

// A module that tolerates any use.
function stubModule() {
  const f = function () {};
  return new Proxy(f, {
    get: (_, k) => (k === Symbol.toPrimitive ? () => 'stub' : k === 'then' ? undefined : stubModule()),
    apply: () => stubModule(),
    construct: () => stubModule(),
  });
}

const origRequire = Module.prototype.require;
Module.prototype.require = function (id) {
  const mine = this && this.filename === FILE;
  if (mine) record('require', 'require', id);
  try {
    return origRequire.call(this, id);
  } catch (e) {
    if (mine) return stubModule();
    throw e;
  }
};
// `createRequire(filename)`: the function it returns resolves from `filename`, not from the
// program, so a call records the file Node resolved (`resolved: true`), not the specifier.
// A request that does not resolve loads nothing and is not recorded.
const origCreateRequire = Module.createRequire;
Module.createRequire = function (filename) {
  const req = origCreateRequire.call(this, filename);
  const wrapped = function (id) {
    const l = line(false);
    let file = null;
    try {
      file = req.resolve(id);
    } catch {}
    if (l && typeof file === 'string' && path.isAbsolute(file)) seen.push({ api: 'createRequire()', kind: 'require', line: l, t: 'string', v: file, resolved: true });
    try {
      return req(id);
    } catch (e) {
      if (l) return stubModule();
      throw e;
    }
  };
  wrapped.resolve = Object.assign(
    (id, options) => {
      const l = line(false);
      const file = req.resolve(id, options);
      if (l && typeof file === 'string' && path.isAbsolute(file)) seen.push({ api: 'createRequire().resolve', kind: 'require', line: l, t: 'string', v: file, resolved: true });
      return file;
    },
    { paths: (id) => req.resolve.paths(id) },
  );
  wrapped.cache = req.cache;
  wrapped.main = req.main;
  wrapped.extensions = req.extensions;
  return wrapped;
};
Module.syncBuiltinESMExports();

const origResolve = Module._resolveFilename;
// The record the resolve hook (below) made for the resolution in progress.
let hooked = null;
Module._resolveFilename = function (request, parent, ...rest) {
  const paths = Boolean(rest[1] && rest[1].paths);
  // A request searched from `paths` is recorded with the file it finds (below): its
  // specifier alone does not say from where, so the hook's record of it is dropped.
  if (paths && hooked) {
    const i = seen.indexOf(hooked);
    if (i >= 0) seen.splice(i, 1);
    hooked = null;
  }
  try {
    const file = origResolve.call(this, request, parent, ...rest);
    // `require.resolve(request, { paths })` in the program: the file found from there.
    if (rest[1] && rest[1].paths && parent && parent.filename === FILE && typeof file === 'string' && path.isAbsolute(file)) {
      const l = line(false, true);
      if (l) seen.push({ api: 'require.resolve', kind: 'require', line: l, t: 'string', v: file, resolved: true });
    }
    return file;
  } catch (e) {
    // `require.resolve(x)` in the program: the specifier is what matters.
    if (parent && parent.filename === FILE && !paths) return path.resolve(path.dirname(FILE), String(request));
    throw e;
  }
};

if (typeof Module.registerHooks === 'function') {
  Module.registerHooks({
    resolve(specifier, context, next) {
      const mine = context.parentURL && context.parentURL.startsWith('file:') && fileURLToPath(context.parentURL) === FILE;
      const dynamic = mine && !STATIC.has(specifier);
      const outer = hooked;
      hooked = null;
      if (dynamic) {
        hooked = { api: 'import', kind: 'dynamic_import', line: line(true), t: 'string', v: specifier };
        seen.push(hooked);
      }
      try {
        return next(specifier, context);
      } catch (e) {
        if (mine) return { url: 'data:text/javascript,export default {}', shortCircuit: true };
        throw e;
      } finally {
        hooked = outer;
      }
    },
  });
}
// Specifiers of the program's static imports (not sites).
const STATIC = new Set();
try {
  const text = fs.readFileSync(FILE, 'utf8');
  for (const m of text.matchAll(/^\s*(?:import|export)\b[^'"\n(]*?from\s*['"]([^'"]+)['"]/gm)) STATIC.add(m[1]);
  for (const m of text.matchAll(/^\s*import\s*['"]([^'"]+)['"]/gm)) STATIC.add(m[1]);
} catch {}

process.on('exit', () => {
  try {
    writeOut(OUT, JSON.stringify(seen));
  } catch {}
});
process.on('uncaughtException', (e) => {
  seen.push({ api: 'crash', kind: 'crash', line: 0, t: 'error', v: String(e && e.message) });
  process.exitCode = 0;
});
process.on('unhandledRejection', (e) => {
  seen.push({ api: 'crash', kind: 'crash', line: 0, t: 'error', v: String(e && e.message) });
});
