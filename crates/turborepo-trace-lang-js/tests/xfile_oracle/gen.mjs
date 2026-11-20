// Program generator of the cross-file binding oracle (see oracle.mjs): a deterministic
// multi-file project from a seed.
//
// A project is a small monorepo: two or three packages below pkgs/, three to six test
// entries, and one to three *helpers*. A helper is a function (or class) in one file with a
// module load whose path depends on its parameters; each helper kind is one of the shapes
// listed in KINDS, with several forms. Every helper gets a few callers: test files, and
// modules between a test and the helper. How a caller reaches the helper is drawn
// independently (TRANSPORTS): a direct import, a namespace, a module in between that passes
// its own parameter on or calls with a literal, a re-export barrel (`barrel`), a way that
// lets the function escape (`escape`), the setup file of the test's package (`setup`).
//
// Six programs in ten are *clean*: ES modules only and no escaping caller, so every caller
// of a helper is known and binding can close its sites. Three in ten have aliases in the
// packages' jest configurations (`alias`): `@shared/x` for modules a helper loads, `@pkgs/x`
// for imports. A few forms exceed what a binder follows (re-export hops, modules a
// parameter passes through, nested return summaries, table combinations).
//
// The generator does not know what a program loads: the oracle learns it by running the
// entries. It only has to make sure the files a call can load exist.
//
// Test and helper modules are written with `import` / `export` or as CommonJS (`style`);
// a module with `import` declarations may still call `require` and use `__dirname`, as
// transpiled test code does (preload.cjs provides both).
import { posix } from 'node:path';

function rng(seed) {
  let a = seed >>> 0 || 1;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const q = (s) => `'${s}'`;
const arr = (xs) => `[${xs.join(', ')}]`;
// The operand of `new`: an expression with a call in it needs parentheses.
const N = (f) => (/^[\w.]+$/.test(f) ? f : `(${f})`);
const indent = (text) =>
  text
    .split('\n')
    .map((l) => (l ? `  ${l}` : l))
    .join('\n');
const block = (body) => indent(Array.isArray(body) ? body.flat().join('\n') : body);
// Arguments with statements that have to run before the call.
const withPre = (args, pre) => Object.assign(args, { pre });

// A module under construction.
class Mod {
  constructor(p, file, style) {
    this.p = p;
    this.file = file;
    this.style = style;
    this.heads = [];
    this.items = [];
    this.tail = [];
    this.listed = [];
    this.names = new Set(['path']);
    this.bound = new Map();
    this.ns = new Map();
    this.sides = new Set();
    this.hasPath = false;
    this.hasDefault = false;
    this.exportForm = style === 'esm' ? p.pick(['inline', 'inline', 'list']) : p.pick(['dot', 'dot', 'mdot', 'object']);
  }
  get esm() {
    return this.style === 'esm';
  }
  get dir() {
    return posix.dirname(this.file);
  }
  local(base) {
    let n = base;
    for (let i = 1; this.names.has(n); i++) n = `${base}${i}`;
    this.names.add(n);
    return n;
  }
  // `dir` relative to the module's directory.
  rel(dir) {
    return posix.relative(this.dir, dir) || '.';
  }
  // The specifier of `file` as this module writes it.
  spec(file, ext = this.p.chance(0.3)) {
    let s = posix.relative(this.dir, file);
    if (!s.startsWith('.')) s = `./${s}`;
    if (ext) return s;
    s = s.replace(/\.js$/, '');
    if (s.endsWith('/index') && this.p.chance(0.6)) s = s.slice(0, -'/index'.length);
    return s;
  }
  path() {
    if (!this.hasPath) {
      this.hasPath = true;
      this.heads.push(this.esm ? this.p.pick(["import path from 'path';", "import * as path from 'node:path';"]) : "const path = require('path');");
    }
    return 'path';
  }
  // The specifier of an import of `file`: in a program with aliases, sometimes the alias of
  // the packages directory.
  importSpec(file) {
    if (this.p.alias && file.startsWith('pkgs/') && this.p.chance(0.25)) {
      this.p.shapes.add('alias');
      return `@${file.replace(/\.js$/, '')}`;
    }
    return this.spec(file);
  }
  // Loads `file` for its effects.
  side(file) {
    if (this.sides.has(file) || file === this.file) return;
    this.sides.add(file);
    const s = this.importSpec(file);
    this.heads.push(this.esm ? `import '${s}';` : `require('${s}');`);
  }
  // An expression for the export `name` of the module `file`.
  imp(file, name, how) {
    const key = `${file}\0${name}`;
    if (this.bound.has(key)) return this.bound.get(key);
    const s = this.importSpec(file);
    how ??= !this.esm ? this.p.pick(['named', 'named', 'alias', 'req', 'inline']) : this.p.clean ? this.p.pick(['named', 'named', 'alias', 'ns']) : this.p.pick(['named', 'named', 'named', 'alias', 'ns', 'req']);
    let e;
    if (how === 'ns' && this.esm) {
      if (!this.ns.has(file)) {
        const n = this.local('ns');
        this.ns.set(file, n);
        this.heads.push(`import * as ${n} from '${s}';`);
      }
      e = `${this.ns.get(file)}.${name}`;
    } else if (how === 'req' || how === 'ns') {
      const o = this.local('m');
      this.heads.push(`const ${o} = require('${s}');`);
      e = `${o}.${name}`;
    } else if (how === 'inline') {
      e = `require('${s}').${name}`;
    } else {
      const base = name === 'default' ? 'dflt' : name;
      const l = this.local(how === 'alias' ? `${base}_` : base);
      if (this.esm) this.heads.push(name === 'default' ? `import ${l} from '${s}';` : l === name ? `import { ${name} } from '${s}';` : `import { ${name} as ${l} } from '${s}';`);
      else this.heads.push(l === name ? `const { ${name} } = require('${s}');` : `const { ${name}: ${l} } = require('${s}');`);
      e = l;
    }
    this.bound.set(key, e);
    return e;
  }
  // A declaration; `exported`: true or the exported name.
  decl(name, text, exported = false) {
    this.names.add(name);
    if (!exported) {
      this.items.push(text);
      return;
    }
    const as = exported === true ? name : exported;
    if (this.esm && as === name && this.exportForm === 'inline') this.items.push(`export ${text}`);
    else {
      this.items.push(text);
      this.expose(name, as);
    }
  }
  // Exports a binding declared before.
  expose(name, as = name) {
    this.names.add(name);
    if (this.esm) this.listed.push(as === name ? name : `${name} as ${as}`);
    else if (this.exportForm === 'dot') this.items.push(`exports.${as} = ${name};`);
    else if (this.exportForm === 'mdot') this.items.push(`module.exports.${as} = ${name};`);
    else this.listed.push(as === name ? name : `${as}: ${name}`);
  }
  fn(name, params, body, { exported = true, form } = {}) {
    form ??= this.p.pick(['decl', 'decl', 'decl', 'arrow', 'expr']);
    const b = block(body);
    const text =
      form === 'decl' ? `function ${name}(${params}) {\n${b}\n}` : form === 'async' ? `async function ${name}(${params}) {\n${b}\n}` : form === 'arrow' ? `const ${name} = (${params}) => {\n${b}\n};` : `const ${name} = function (${params}) {\n${b}\n};`;
    this.decl(name, text, exported);
  }
  // Also the default export (ES modules only).
  alsoDefault(name) {
    if (this.hasDefault) return this.hasDefault === name;
    this.hasDefault = name;
    this.tail.push(`export default ${name};`);
    return true;
  }
  blocks() {
    return [];
  }
  render() {
    const out = [...this.heads, ...this.items, ...this.blocks(), ...this.tail];
    if (this.listed.length) out.push(this.esm ? `export { ${this.listed.join(', ')} };` : `module.exports = { ${this.listed.join(', ')} };`);
    return `${out.join('\n')}\n`;
  }
}

// A test entry: the calls it makes go into test blocks.
class Test extends Mod {
  constructor(p, file, style) {
    super(p, file, style);
    this.calls = [];
    this.form = p.pick(['test', 'test', 'describe', 'top', 'before']);
  }
  call(...stmts) {
    this.calls.push(stmts.flat());
  }
  blocks() {
    const name = posix.basename(this.file, '.test.js');
    const plain = `test('${name}', () => {\n  expect(1).toBe(1);\n});`;
    if (!this.calls.length) return [plain];
    if (this.form === 'describe') {
      const its = this.calls.map((g, i) => `it('case ${i}', () => {\n${block(g)}\n});`);
      return [`describe('${name}', () => {\n${indent(its.join('\n'))}\n});`];
    }
    if (this.form === 'top') return [...this.calls.flat(), plain];
    if (this.form === 'before') return [`beforeAll(() => {\n${block(this.calls)}\n});`, plain];
    return [`test('${name}', () => {\n${block(this.calls)}\n});`];
  }
}

class Proj {
  constructor(seed) {
    this.r = rng(seed);
    this.n = 0;
    this.raw = {};
    this.mods = [];
    this.shapes = new Set();
    // A clean program has ES modules only and no caller that lets a helper escape: every
    // caller is known, so binding can close the helpers' sites.
    this.clean = this.chance(0.6);
    // The packages' jest configurations map `@shared/x` to <root>/shared/x and `@pkgs/x`
    // to <root>/pkgs/x.
    this.alias = this.chance(0.3);
    this.setups = {};
    this.pkgs = ['p1', 'p2', 'p3'].slice(0, 2 + this.int(2));
    this.tests = [];
    const count = 3 + this.int(4);
    for (let i = 0; i < count; i++) {
      const pkg = this.pkgs[i % this.pkgs.length];
      const where = this.pick(['test/', 'test/', 'src/__tests__/', '']);
      this.tests.push(new Test(this, `pkgs/${pkg}/${where}t${i}.test.js`, this.style()));
    }
  }
  int(n) {
    return Math.floor(this.r() * n);
  }
  pick(xs) {
    return xs[this.int(xs.length)];
  }
  chance(x) {
    return this.r() < x;
  }
  style() {
    return this.clean || this.chance(0.75) ? 'esm' : 'cjs';
  }
  id() {
    return this.n++;
  }
  has(file) {
    return file in this.raw || this.mods.some((m) => m.file === file);
  }
  mod(file, style = this.style()) {
    const m = new Mod(this, file, style);
    this.mods.push(m);
    return m;
  }
  // The directory of one helper and what it loads.
  area(tag) {
    return `pkgs/${this.pick(this.pkgs)}/${tag}${this.id()}`;
  }
  names() {
    return this.chance(0.3) ? ['a', 'b', 'c', 'd'] : ['a', 'b', 'c'];
  }
  // A module that can be loaded; one in four loads a module of its own.
  target(file) {
    if (this.has(file)) return;
    const base = posix.basename(file, '.js');
    const form = this.pick(['cjs', 'cjs', 'dot', 'esm']);
    let dep = '';
    if (this.chance(0.25)) {
      const depFile = posix.join(posix.dirname(file), '..', 'deps', `${base}-dep.js`);
      this.raw[depFile] ??= `module.exports = { dep: '${base}' };\n`;
      dep = form === 'esm' ? `import '../deps/${base}-dep.js';\n` : `require('../deps/${base}-dep');\n`;
    }
    this.raw[file] = dep + (form === 'cjs' ? `module.exports = { name: '${base}' };\n` : form === 'dot' ? `exports.name = '${base}';\n` : `export const name = '${base}';\nexport default { name };\n`);
  }
  targets(dir, names) {
    for (const n of names) this.target(`${dir}/${n}.js`);
  }
  file(file, text) {
    this.raw[file] ??= text;
  }
  // The setup file of a package: the test runner loads it before every test of the package.
  setup(pkg) {
    this.shapes.add('setup');
    this.setups[pkg] ??= this.mod(`pkgs/${pkg}/jest.setup.js`);
    return this.setups[pkg];
  }
  // A directory below <root>/shared for the modules of one helper and how a module names
  // it, when the program has the alias and the draw says so; else null.
  shared(area) {
    if (!this.alias || !this.chance(0.6)) return null;
    this.shapes.add('alias');
    const name = posix.basename(area);
    return { dir: `shared/${name}`, spec: `@shared/${name}` };
  }
  finish() {
    const files = { ...this.raw };
    files['package.json'] = '{"name":"root","version":"1.0.0","private":true,"workspaces":["pkgs/*"]}\n';
    files['README.md'] = 'x\n';
    for (const pkg of this.pkgs) {
      files[`pkgs/${pkg}/package.json`] = `{"name":"${pkg}","version":"1.0.0"}\n`;
      const options = [];
      if (this.alias) options.push("moduleNameMapper: { '^@shared/(.*)$': '<rootDir>/../../shared/$1', '^@pkgs/(.*)$': '<rootDir>/../$1' }");
      if (this.setups[pkg]) options.push(this.pick(["setupFiles: ['<rootDir>/jest.setup.js']", "setupFilesAfterEnv: ['<rootDir>/jest.setup.js']"]));
      if (this.chance(0.5)) options.push("testEnvironment: 'node'");
      files[`pkgs/${pkg}/jest.config.js`] = options.length ? `module.exports = {\n  ${options.join(',\n  ')},\n};\n` : 'module.exports = {};\n';
    }
    for (const m of [...this.mods, ...this.tests]) files[m.file] = m.render();
    return { files, entries: this.tests.map((t) => t.file), shapes: [...this.shapes].sort() };
  }
}

// ---- helper kinds ----
//
// A kind returns the callables it made: { mod, name, variants, invoke?, varargs? }.
//   variants  functions of the calling module that return the argument expressions of one
//             call (with `pre`: statements that run before it)
//   invoke    how a caller uses the export `f` with the arguments (default: `f(args)`)
//   varargs   the number of arguments matters (rest parameters, `arguments`)

const one = (names, f = q) => names.map((n) => (m) => [f(n, m)]);
// One of the forms; `prefer` lists the ones drawn more often (the ones whose load has no
// part in the helper: the value is the caller's alone).
const draw = (p, forms, prefer) => forms[prefer && p.chance(0.5) ? p.pick(prefer) : p.pick(Object.keys(forms))]();

// `lit`: the plain parameter-path load.
function kLit(p) {
  const d = p.area('lit');
  const sub = p.pick(['mods', 'handlers', 'plugins']);
  const names = p.names();
  p.targets(`${d}/${sub}`, names);
  const h = p.mod(`${d}/${p.pick(['helper', 'load', 'index'])}.js`);
  // A whole specifier from the caller: relative to the helper, or an alias.
  let shared = null;
  const whole = (n, ext = '') => (shared ? q(`${shared.spec}/${n}${ext}`) : q(`./${sub}/${n}${ext}`));
  const aliased = () => {
    shared = p.shared(d);
    if (shared) p.targets(shared.dir, names);
  };
  const abs = (n, m) =>
    p.pick([
      () => `${m.path()}.join(__dirname, ${q(m.rel(`${d}/${sub}`))}, ${q(`${n}.js`)})`,
      () => `require.resolve(${q(m.spec(`${d}/${sub}/${n}.js`))})`,
      () => `__dirname + ${q(`/${m.rel(`${d}/${sub}`)}/${n}.js`)}`,
    ])();
  const [params, body, variants] = draw(p, {
    concat: () => ['name', `return require('./${sub}/' + name);`, one(names)],
    template: () => ['name', `return require(\`./${sub}/\${name}\`);`, one(names)],
    ext: () => ['name', `return require('./${sub}/' + name + '.js');`, one(names)],
    join: () => ['name', `return require(${h.path()}.join(__dirname, '${sub}', name));`, one(names)],
    resolveJoin: () => ['name', `return require(${h.path()}.resolve(__dirname, '${sub}', name + '.js'));`, one(names)],
    imp: () => ['name', `return import('./${sub}/' + name + '.js');`, one(names)],
    impTemplate: () => ['name', `return import(\`./${sub}/\${name}.js\`);`, one(names)],
    spec: () => {
      aliased();
      return ['spec', 'return require(spec);', one(names, (n) => whole(n))];
    },
    specImp: () => {
      aliased();
      return ['spec', 'return import(spec);', one(names, (n) => whole(n, '.js'))];
    },
    abs: () => ['file', 'return require(file);', one(names, abs)],
    absImp: () => ['file', 'return import(file);', one(names, abs)],
    resolve: () => ['name', [`const file = require.resolve('./${sub}/' + name);`, 'return require(file);'], one(names)],
    two: () => ['dir, name', "return require('./' + dir + '/' + name);", names.map((n) => () => [q(sub), q(n)])],
    inner: () => {
      h.fn('inner', 'p', 'return require(p);', { exported: false });
      return ['name', `return inner('./${sub}/' + name);`, one(names)];
    },
    loop: () => ['list', `return list.map((n) => require('./${sub}/' + n));`, [() => [arr(names.slice(0, 2).map(q))], () => [arr([q(names[2])])], () => ['[]']]],
    dirOnly: () => ['dir', `return require('./' + dir + '/${names[0]}');`, [() => [q(sub)]]],
    dirName: () => ['dir, name', "return require(dir + '/' + name);", names.map((n) => () => [q(`./${sub}`), q(n)])],
    baseJoin: () => ['base, name', `return require(${h.path()}.join(base, name));`, names.map((n) => (m) => [`${m.path()}.join(__dirname, ${q(m.rel(`${d}/${sub}`))})`, q(n)])],
    specList: () => {
      aliased();
      return ['specs', 'return specs.map((s) => require(s));', [() => [arr(names.slice(0, 2).map((n) => whole(n)))], () => [arr([whole(names[2])])]]];
    },
  }, ['spec', 'specImp', 'abs', 'absImp', 'dirName', 'baseJoin', 'specList']);
  h.fn('load', params, body);
  return [{ mod: h, name: 'load', variants }];
}

// `opaque`: a string method between the parameter and the load.
function kOpaque(p) {
  const d = p.area('opq');
  const names = p.names();
  p.targets(`${d}/mods`, [...names, ...names.map((n) => n.toUpperCase()), ...names.map((n) => n + n)]);
  const h = p.mod(`${d}/helper.js`);
  const rev = (s) => [...s].reverse().join('');
  const [body, f] = draw(p, {
    lower: () => ["return require('./mods/' + name.toLowerCase());", (n) => q(n.toUpperCase())],
    specLower: () => ['return require(name.toLowerCase());', (n) => q(`./Mods/${n.toUpperCase()}`)],
    trim: () => ['return require(name.trim());', (n) => q(` ./mods/${n} `)],
    replace: () => ["return require(name.replace('@', './mods/'));", (n) => q(`@${n}`)],
    replaceRe: () => ["return require(name.replace(/^@\\//, './mods/'));", (n) => q(`@/${n}`)],
    suffix: () => ["return require('./mods/' + name.replace('-x', ''));", (n) => q(`${n}-x`)],
    slice: () => ['return require(`./mods/${name.slice(1)}`);', (n) => q(`x${n}`)],
    sliceSpec: () => ['return require(name.slice(1));', (n) => q(`x./mods/${n}`)],
    split: () => ["return require(name.split('|')[0]);", (n) => q(`./mods/${n}|zzz`)],
    concat: () => ["return require(name.concat('.js'));", (n) => q(`./mods/${n}`)],
    upper: () => ["return require('./mods/' + name.toUpperCase());", (n) => q(n)],
    repeat: () => ["return require('./mods/' + name.repeat(2));", (n) => q(n)],
    charAt: () => ["return require('./mods/' + name.charAt(0));", (n) => q(`${n}zz`)],
    at: () => ["return require('./mods/' + name.at(-1));", (n) => q(`zz${n}`)],
    reverse: () => ["return require(name.split('').reverse().join(''));", (n) => q(rev(`./mods/${n}`))],
    json: () => ['return require(JSON.parse(name).file);', (n) => q(`{"file":"./mods/${n}"}`)],
    string: () => ['return require(String(name));', (n) => q(`./mods/${n}`)],
    tpl: () => ['return require(`${name}`);', (n) => q(`./mods/${n}`)],
    normalize: () => [`return require('./' + ${h.path()}.normalize(name));`, (n) => q(`mods/../mods/${n}`)],
    decode: () => ['return require(decodeURIComponent(name));', (n) => q(`.%2Fmods%2F${n}`)],
    padEnd: () => ["return require(name.padEnd(name.length + 3, '.js'));", (n) => q(`./mods/${n}`)],
    impLower: () => ["return import('./mods/' + name.toLowerCase() + '.js');", (n) => q(n.toUpperCase())],
    impTrim: () => ['return import(name.trim());', (n) => q(` ./mods/${n}.js `)],
    impReplace: () => ["return import(name.replace('@', './mods/'));", (n) => q(`@${n}.js`)],
  }, ['specLower', 'trim', 'replace', 'replaceRe', 'sliceSpec', 'split', 'concat', 'reverse', 'json', 'string', 'tpl', 'decode', 'padEnd', 'impTrim', 'impReplace']);
  h.fn('load', 'name', body);
  return [{ mod: h, name: 'load', variants: one(names, f) }];
}

// `opts`: option objects, destructuring, defaults, rest and spread, missing arguments.
function kOpts(p) {
  const d = p.area('opts');
  const names = p.names();
  const [a, b, c] = names;
  p.targets(`${d}/mods`, names);
  const h = p.mod(`${d}/helper.js`);
  // The caller passes the whole specifier, or a name below a directory of the helper.
  const whole = p.chance(0.6);
  const L = (x) => (whole ? x : `'./mods/' + ${x}`);
  const V = (n) => q(whole ? `./mods/${n}` : n);
  const obj = (f) => names.map((n) => () => [f(n)]);
  const v = (...lists) => lists.map((l) => () => l);
  let opts = {};
  const [params, body, variants] = draw(p, {
    obj: () => ['opts', `return require(${L('opts.name')});`, [...obj((n) => `{ name: ${V(n)} }`), () => [`{ name: ${V(a)}, extra: 1 }`]]],
    destr: () => ['{ dir, name }', "return require('./' + dir + '/' + name);", [...obj((n) => `{ dir: 'mods', name: ${q(n)} }`), () => [`{ name: ${q(b)}, dir: 'mods' }`]]],
    destrWhole: () => ['{ dir, name }', "return require(dir + '/' + name);", [...obj((n) => `{ dir: './mods', name: ${q(n)} }`), () => [`{ name: ${q(b)}, dir: './mods' }`]]],
    destrDefault: () => [`{ name = ${V(a)} } = {}`, `return require(${L('name')});`, v([], ['{}'], [`{ name: ${V(b)} }`], ['undefined'], ['{ name: undefined }'])],
    paramDefault: () => [`name = ${V(a)}`, `return require(${L('name')});`, v([], [V(b)], ['undefined'], [V(c)])],
    nullish: () => ['opts', `return require(${L(`(opts.name ?? ${V(a)})`)});`, v(['{}'], [`{ name: ${V(b)} }`], ['{ name: undefined }'], ['{ name: null }'])],
    or: () => ['opts', `return require(${L(`(opts.name || ${V(a)})`)});`, v(['{}'], ["{ name: '' }"], [`{ name: ${V(b)} }`], ['{ name: 0 }'])],
    guard: () => ['opts', [`const name = (opts && opts.name) || ${V(a)};`, `return require(${L('name')});`], v([], ['null'], [`{ name: ${V(c)} }`])],
    optChain: () => ['opts', `return require(${L(`(opts?.name ?? ${V(a)})`)});`, v([], [`{ name: ${V(c)} }`], ['undefined'])],
    rest: () => {
      opts = { varargs: true };
      return ['...names', `return names.map((n) => require(${L('n')}));`, v([V(a), V(b)], [`...[${V(a)}, ${V(c)}]`], [V(c)], [])];
    },
    restAfter: () => {
      opts = { varargs: true };
      return ['first, ...more', `return [first, ...more].map((n) => require(${L('n')}));`, v([V(a), V(b)], [V(a), `...[${V(c)}]`], [V(c)])];
    },
    spread: () => [
      'dir, name',
      "return require(dir + '/' + name);",
      [
        () => [`...['./mods', ${q(a)}]`],
        (m) => {
          const args = m.local('args');
          m.items.push(`const ${args} = ['./mods', ${q(b)}];`);
          return [`...${args}`];
        },
        () => [q('./mods'), `...[${q(c)}]`],
      ],
    ],
    missing: () => ['dir, name', `return require((dir || './mods') + '/' + (name || ${q(a)}));`, v([], [q('./mods')], ['undefined', q(b)], [q('./mods'), q(c)])],
    args: () => {
      opts = { varargs: true, form: 'decl' };
      return ['', `return require(${L('arguments[0]')});`, v([V(a)], [V(b), q('x')])];
    },
    argsLength: () => {
      opts = { varargs: true, form: 'expr' };
      return ['', `return require(${L(`(arguments.length > 1 ? arguments[1] : ${V(a)})`)});`, v([q('x')], [q('x'), V(b)], [])];
    },
    nested: () => ['opts', `return require(${L('opts.target.name')});`, obj((n) => `{ target: { name: ${V(n)} } }`)],
    pair: () => ['pair', "return require(pair[0] + '/' + pair[1]);", obj((n) => `['./mods', ${q(n)}]`)],
    arrDestr: () => ['[dir, name]', "return require(dir + '/' + name);", obj((n) => `['./mods', ${q(n)}]`)],
    objRest: () => ['{ name, ...rest }', "return require(rest.dir + '/' + name);", obj((n) => `{ name: ${q(n)}, dir: './mods' }`)],
    computedKey: () => ['opts, key', `return require(${L('opts[key]')});`, v([`{ x: ${V(a)}, y: ${V(b)} }`, q('x')], [`{ x: ${V(a)}, y: ${V(b)} }`, q('y')], [`{ z: ${V(c)} }`, q('z')])],
    laterDefault: () => ["name, dir = './mods'", "return require(dir + '/' + name);", v([q(a)], [q(b), q('./mods')], [q(c), 'undefined'])],
    defaultFromParam: () => ['name, file = name', `return require(${L('file')});`, v([V(a)], [q('x'), V(b)])],
    impOpts: () => ['{ name }', whole ? 'return import(name);' : "return import('./mods/' + name + '.js');", obj((n) => `{ name: ${q(whole ? `./mods/${n}.js` : n)} }`)],
    configPath: () => ['options', ['const config = require(options.configPath);', 'return JSON.stringify({ config, tags: options.tags });'], obj((n) => `{ configPath: ${q(`./mods/${n}`)}, tags: [] }`)],
    configPathAbs: () => [
      'options',
      ['const config = require(options.configPath);', 'return { config, tags: options.tags };'],
      names.map((n) => (m) => [`{ configPath: ${m.path()}.join(__dirname, ${q(m.rel(`${d}/mods`))}, ${q(`${n}.js`)}), tags: [] }`]),
    ],
  });
  h.fn('load', params, body, { form: opts.form });
  return [{ mod: h, name: 'load', variants, varargs: opts.varargs }];
}

// `table`: a lookup table read by key; the value also leaves the helper in other ways.
function kTable(p) {
  const d = p.area('tbl');
  const names = p.names();
  p.targets(`${d}/mods`, [...names, 'z']);
  const h = p.mod(`${d}/helper.js`);
  // The table's values: relative to the helper, or an alias.
  const shared = p.shared(d);
  if (shared) p.targets(shared.dir, names);
  const entries = (key = (n) => n, value = (n) => q(shared ? `${shared.spec}/${n}` : `./mods/${n}`)) => names.map((n) => `  ${key(n)}: ${value(n)},`).join('\n');
  const table = (text = `{\n${entries()}\n}`, exported = false) => h.decl('T', `const T = ${text};`, exported);
  const keys = one(names);
  const out = [];
  const add = (name, variants, extra = {}) => out.push({ mod: h, name, variants, ...extra });
  const sibling = (name) => p.mod(`${d}/${name}.js`);
  draw(p, {
    plain: () => {
      table();
      h.fn('load', 'key', 'return require(T[key]);');
      add('load', [...keys, () => [`${q('k')}.replace('k', ${q(names[0])})`]]);
    },
    imp: () => {
      table(`{\n${entries(undefined, (n) => q(`./mods/${n}.js`))}\n}`);
      h.fn('load', 'key', 'return import(T[key]);');
      add('load', keys);
    },
    leakFn: () => {
      table();
      h.fn('load', 'key', 'return require(T[key]);');
      h.fn('pathOf', 'key', 'return T[key];');
      const s = sibling('other');
      s.fn('viaPath', 'key', `return require(${s.imp(h.file, 'pathOf')}(key));`);
      add('load', keys);
      out.push({ mod: s, name: 'viaPath', variants: keys });
    },
    leakValues: () => {
      table();
      h.fn('load', 'key', 'return require(T[key]);');
      h.fn('all', '', p.pick(['return Object.values(T).map((f) => require(f));', 'return Object.keys(T).map((k) => require(T[k]));', ['const out = [];', 'for (const k in T) out.push(require(T[k]));', 'return out;']]));
      add('load', keys);
      add('all', [() => []]);
    },
    leakEntries: () => {
      table();
      h.fn('except', 'skip', 'return Object.entries(T).filter(([k]) => k !== skip).map(([, f]) => require(f));');
      add('except', keys);
    },
    third: () => {
      table();
      h.fn('third', 'file', 'return require(file);', { exported: false });
      h.fn('load', 'key', 'return third(T[key]);');
      add('load', keys);
    },
    thirdFile: () => {
      table();
      const s = sibling('loader');
      s.fn('req', 'file', 'return require(file);');
      h.fn('load', 'key', `return ${h.imp(s.file, 'req')}(T[key]);`);
      add('load', keys);
    },
    handoff: () => {
      table();
      const s = sibling('loader');
      s.fn('req', 'file', 'return require(file);');
      h.fn('pathOf', 'key', 'return T[key];');
      add('pathOf', keys, { invoke: (f, args, m) => `${m.imp(s.file, 'req')}(${f}(${args.join(', ')}))` });
    },
    numeric: () => {
      table(`{\n${names.map((n, i) => `  ${i + 1}: ${q(`./mods/${n}`)},`).join('\n')}\n}`);
      h.fn('load', 'key', 'return require(T[key]);');
      add('load', [() => ['1'], () => [q('2')], () => ['1 + 2'], () => [`${names.length}`]]);
    },
    array: () => {
      table(arr(names.map((n) => q(`./mods/${n}`))));
      h.fn('load', 'i', 'return require(T[i]);');
      add('load', [() => ['0'], () => ['1'], () => [`${names.length - 1}`], () => [q('0')]]);
    },
    computedKeys: () => {
      table(`{\n${entries((n) => `k${n}`)}\n}`);
      h.fn('load', 'key', 'return require(T[key]);');
      add('load', [
        () => [`'k' + ${q(names[0])}`],
        () => [`['k${names[0]}', 'k${names[1]}'][1]`],
        () => [`'K${names[2].toUpperCase()}'.toLowerCase()`],
        () => [`\`k\${${q(names[1])}}\``],
        () => [`Object.keys({ k${names[2]}: 1 })[0]`],
        (m) => {
          const k = m.local('KEYS');
          m.items.push(`const ${k} = { first: 'k${names[0]}', second: 'k${names[1]}' };`);
          return [`${k}.second`];
        },
      ]);
    },
    unknownKey: () => {
      table(`{\n  k0: ${q(`./mods/${names[0]}`)},\n  k1: ${q(`./mods/${names[1]}`)},\n  other: ${q(`./mods/${names[2]}`)},\n}`);
      h.fn('load', 'key', 'return require(T[key]);');
      add('load', [() => ["'k' + process.env.FLOW_K0"], () => [q('other')], () => ["process.env.FLOW_K1 === '1' ? 'k0' : 'other'"], () => ['`k${process.env.FLOW_K2}`']]);
    },
    exported: () => {
      table(undefined, true);
      h.fn('load', 'key', 'return require(T[key]);');
      // A third file adds an entry.
      const ext = sibling('ext');
      ext.items.push(`${ext.imp(h.file, 'T')}.extra = './mods/z';`);
      // Another one reads the table itself.
      const s = sibling('direct');
      s.fn('direct', 'key', `return require(${s.imp(h.file, 'T')}[key]);`);
      add('load', [
        ...keys,
        (m) => {
          m.side(ext.file);
          return [q('extra')];
        },
      ]);
      out.push({ mod: s, name: 'direct', variants: keys });
    },
    map: () => {
      table(`new Map([\n${names.map((n) => `  [${q(n)}, ${q(`./mods/${n}`)}],`).join('\n')}\n])`);
      h.fn('load', 'key', 'return require(T.get(key));');
      add('load', keys);
    },
    fallback: () => {
      table();
      h.fn('load', 'key', p.pick(['return require(T[key] || T.a);', "return require(T[key] ?? './mods/z');", "return require(key in T ? T[key] : './mods/z');", "return require(Object.hasOwn(T, key) ? T[key] : './mods/z');"]));
      add('load', [...keys, () => [q('nope')]]);
    },
    nested: () => {
      table(`{\n  g1: { ${names[0]}: ${q(`./mods/${names[0]}`)} },\n  g2: { ${names[1]}: ${q(`./mods/${names[1]}`)}, ${names[2]}: ${q(`./mods/${names[2]}`)} },\n}`);
      h.fn('load', 'group, key', 'return require(T[group][key]);');
      add('load', [() => [q('g1'), q(names[0])], () => [q('g2'), q(names[1])], () => [q('g2'), q(names[2])]]);
    },
    objValue: () => {
      table(`{\n${entries(undefined, (n) => `{ file: ${q(`./mods/${n}`)} }`)}\n}`);
      h.fn('load', 'key', p.pick(['return require(T[key].file);', ['const { file } = T[key];', 'return require(file);']]));
      add('load', keys);
    },
    joined: () => {
      table(`{\n${entries((n) => `k${n}`, (n) => q(n))}\n}`);
      h.fn('load', 'key', p.pick(["return require('./mods/' + T[key]);", `return require(${h.path()}.join(__dirname, 'mods', T[key]));`]));
      add('load', one(names, (n) => q(`k${n}`)));
    },
    many: () => {
      table();
      h.fn('loadMany', 'keys', 'return keys.map((k) => require(T[k]));');
      add('loadMany', [() => [arr(names.slice(0, 2).map(q))], () => [arr([q(names[2])])], () => ['[]']]);
    },
    otherFile: () => {
      const t = sibling('table');
      t.decl('T', `const T = {\n${entries()}\n};`, true);
      h.fn('load', 'key', `return require(${h.imp(t.file, 'T')}[key]);`);
      add('load', keys);
    },
    destructured: () => {
      table();
      h.fn('load', 'key', ['const { [key]: file } = T;', 'return require(file);']);
      add('load', keys);
    },
    combinations: () => {
      // More combinations than a binder enumerates: two tables read with keys nobody knows.
      const row = (f) => Array.from({ length: 150 }, (_, i) => `  k${i}: ${q(f(i))},`).join('\n');
      h.decl('DIRS', `const DIRS = {\n${row((i) => `./mods/d${i}`)}\n};`);
      h.decl('NAMES', `const NAMES = {\n${row((i) => `n${i}`)}\n};`);
      h.fn('load', 'x, y', "return require(DIRS[x] + '/' + NAMES[y]);");
      p.targets(`${d}/mods/d0`, ['n0', 'n1']);
      p.targets(`${d}/mods/d1`, ['n0', 'n1']);
      add('load', [() => ["'k' + process.env.FLOW_K0", "'k' + process.env.FLOW_K1"], () => [q('k0'), q('k1')], () => [q('k1'), "'k' + process.env.FLOW_K2"]]);
    },
    frozen: () => {
      table(`Object.freeze({\n${entries()}\n})`);
      h.fn('load', 'key', 'return require(T[key]);');
      add('load', keys);
    },
  });
  return out;
}

// `ret`: return summaries and constants of other files.
function kRet(p) {
  const d = p.area('ret');
  const names = p.names();
  p.targets(`${d}/mods`, [...names, 'z', 'x']);
  p.targets(`${d}/other`, names);
  const h = p.mod(`${d}/helper.js`);
  const keys = one(names);
  const out = [];
  const add = (name, variants, extra = {}) => out.push({ mod: h, name, variants, ...extra });
  const lib = (name, style) => p.mod(`${d}/${name}.js`, style);
  // An import whose value is read at call time (a destructured `require` is a copy).
  const live = (m, file, name) => m.imp(file, name, m.esm ? undefined : 'req');
  // The helper's import of `name` from the module `l`. One in four goes through a barrel
  // that also has `export *` of a module with an export of the same name: the explicit
  // re-export wins, whatever the order.
  const from = (l, name) => {
    if (!l.esm || !p.chance(0.25)) return h.imp(l.file, name);
    p.shapes.add('barrel');
    const id = p.id();
    p.target(`${d}/decoys/never.js`);
    p.file(`${d}/decoy${id}.js`, `export function ${name}() {\n  return './decoys/never';\n}\nexport const other${id} = 1;\n`);
    const explicit = `export { ${name} } from './${posix.basename(l.file, '.js')}';`;
    const star = `export * from './decoy${id}';`;
    p.file(`${d}/barrel${id}.js`, `${p.chance(0.5) ? `${star}\n${explicit}` : `${explicit}\n${star}`}\n`);
    return h.imp(`${d}/barrel${id}.js`, name);
  };
  draw(p, {
    pick: () => {
      const l = lib('paths');
      l.fn('pick', 'name', p.pick(["return './mods/' + name;", 'return `./mods/${name}.js`;', ["if (name === 'x') return './mods/z';", "return './mods/' + name;"], ["const dir = './mods';", "return dir + '/' + name;"]]));
      h.fn('load', 'name', `return require(${from(l, 'pick')}(name));`);
      add('load', [...keys, () => [q('x')]]);
    },
    pickAbs: () => {
      const l = lib('paths');
      l.fn('pick', 'name', `return ${l.path()}.join(__dirname, 'mods', name);`);
      if (p.chance(0.5)) {
        h.fn('load', 'name', `return require(${from(l, 'pick')}(name));`);
        add('load', keys);
      } else {
        // The caller loads what the function returns.
        out.push({ mod: l, name: 'pick', variants: keys, invoke: (f, args) => `require(${f}(${args.join(', ')}))` });
      }
    },
    pickObject: () => {
      const l = lib('paths');
      l.fn('pick', 'name', "return { file: './mods/' + name, name };");
      h.fn('load', 'name', p.pick([`return require(${from(l, 'pick')}(name).file);`, [`const { file } = ${from(l, 'pick')}(name);`, 'return require(file);']]));
      add('load', keys);
    },
    // State of another module that is the whole specifier.
    wholeLet: () => {
      const l = lib('config', 'esm');
      l.items.push(`export let file = './mods/${names[0]}';`, 'export function setFile(f) {\n  file = f;\n}');
      h.fn('load', '', `return require(${live(h, l.file, 'file')});`);
      add('load', [() => [], ...names.slice(1).map((n) => (m) => withPre([], [`${m.imp(l.file, 'setFile')}(${q(`./mods/${n}`)});`]))]);
    },
    wholeGetter: () => {
      const l = lib('config');
      l.items.push(`let file = './mods/${names[0]}';`);
      l.fn('setFile', 'f', 'file = f;');
      l.fn('getFile', '', 'return file;');
      h.fn('load', '', `return require(${from(l, 'getFile')}());`);
      add('load', [() => [], ...names.slice(1).map((n) => (m) => withPre([], [`${m.imp(l.file, 'setFile')}(${q(`./mods/${n}`)});`]))]);
    },
    wholeConfig: () => {
      const l = lib('config');
      l.decl('CONFIG', `const CONFIG = { file: './mods/${names[0]}' };`, true);
      const third = lib('third');
      third.items.push(`${third.imp(l.file, 'CONFIG')}.file = './mods/${names[1]}';`);
      h.fn('load', '', `return require(${h.imp(l.file, 'CONFIG')}.file);`);
      add('load', [
        () => [],
        (m) => {
          m.side(third.file);
          return [];
        },
        (m) => withPre([], [`${m.imp(l.file, 'CONFIG')}.file = './mods/${names[2]}';`]),
      ]);
    },
    wholePick: () => {
      const l = lib('paths');
      l.decl('MAP', `const MAP = {\n${names.map((n) => `  ${n}: './mods/${n}',`).join('\n')}\n};`);
      const fallback = p.chance(0.4);
      l.fn('pick', 'key', fallback ? "return MAP[key] || './mods/z';" : p.pick(['return MAP[key];', ['const file = MAP[key];', 'return file;']]));
      h.fn('load', 'key', `return require(${from(l, 'pick')}(key));`);
      add('load', fallback ? [...keys, () => [q('nope')]] : keys);
    },
    wholeList: () => {
      const l = lib('config');
      l.decl('LIST', `const LIST = ['./mods/${names[0]}'];`, true);
      l.fn('add', 'file', 'LIST.push(file);');
      h.fn('loadAll', '', `return ${h.imp(l.file, 'LIST')}.map((f) => require(f));`);
      add('loadAll', [() => [], ...names.slice(1).map((n) => (m) => withPre([], [`${m.imp(l.file, 'add')}(${q(`./mods/${n}`)});`]))]);
    },
    deepChain: () => {
      let prev = lib('c0');
      prev.fn('f0', '', "return './mods';");
      for (let i = 1; i < 7; i++) {
        const next = lib(`c${i}`);
        next.fn(`f${i}`, '', `return ${next.imp(prev.file, `f${i - 1}`)}();`);
        prev = next;
      }
      h.fn('load', 'name', `return require(${h.imp(prev.file, 'f6')}() + '/' + name);`);
      add('load', keys);
    },
    pickChain: () => {
      const base = lib('base');
      base.fn('base', '', "return './mods';");
      const l = lib('paths');
      l.fn('pick', 'name', `return ${l.imp(base.file, 'base')}() + '/' + name;`);
      h.fn('load', 'name', `return require(${from(l, 'pick')}(name));`);
      add('load', keys);
    },
    constant: () => {
      const l = lib('config');
      l.decl('DIR', "const DIR = './mods';", true);
      h.fn('load', 'name', `return require(${from(l, 'DIR')} + '/' + name);`);
      add('load', keys);
    },
    constantFile: () => {
      const l = lib('config');
      l.decl('DEFAULT', `const DEFAULT = ${l.path()}.join(__dirname, 'mods', '${names[0]}.js');`, true);
      h.fn('load', '', `return require(${from(l, 'DEFAULT')});`);
      add('load', [() => []]);
    },
    config: () => {
      const l = lib('config');
      l.decl('CONFIG', "const CONFIG = { dir: './mods', ext: '.js' };", true);
      h.fn('load', 'name', `return require(${h.imp(l.file, 'CONFIG')}.dir + '/' + name + ${h.imp(l.file, 'CONFIG')}.ext);`);
      add('load', keys);
    },
    exportedLet: () => {
      const l = lib('config', 'esm');
      l.items.push("export let dir = './mods';", 'export function setDir(d) {\n  dir = d;\n}');
      h.fn('load', 'name', `return require(${live(h, l.file, 'dir')} + '/' + name);`);
      add('load', [...keys, ...names.slice(0, 2).map((n) => (m) => withPre([q(n)], [`${m.imp(l.file, 'setDir')}('./other');`]))]);
    },
    getter: () => {
      const l = lib('config');
      l.items.push("let dir = './mods';");
      l.fn('setDir', 'd', 'dir = d;');
      l.fn('getDir', '', 'return dir;');
      h.fn('load', 'name', `return require(${from(l, 'getDir')}() + '/' + name);`);
      add('load', [...keys, ...names.slice(0, 2).map((n) => (m) => withPre([q(n)], [`${m.imp(l.file, 'setDir')}('./other');`]))]);
    },
    mutatedByThird: () => {
      const l = lib('config');
      l.decl('CONFIG', "const CONFIG = { dir: './mods' };", true);
      const third = lib('third');
      const viaCall = p.chance(0.5);
      if (viaCall) third.fn('useOther', '', `${third.imp(l.file, 'CONFIG')}.dir = './other';`);
      else third.items.push(`${third.imp(l.file, 'CONFIG')}.dir = './other';`);
      h.fn('load', 'name', `return require(${h.imp(l.file, 'CONFIG')}.dir + '/' + name);`);
      add('load', [
        ...keys,
        ...names.slice(0, 2).map((n) => (m) => {
          if (viaCall) return withPre([q(n)], [`${m.imp(third.file, 'useOther')}();`]);
          m.side(third.file);
          return [q(n)];
        }),
      ]);
    },
    listPush: () => {
      const l = lib('config');
      l.decl('LIST', `const LIST = ['./mods/${names[0]}'];`, true);
      const third = lib('third');
      third.items.push(`${third.imp(l.file, 'LIST')}.push('./mods/${names[1]}');`);
      h.fn('loadAll', '', `return ${h.imp(l.file, 'LIST')}.map((f) => require(f));`);
      add('loadAll', [
        () => [],
        (m) => {
          m.side(third.file);
          return [];
        },
        (m) => withPre([], [`${m.imp(l.file, 'LIST')}.push('./mods/${names[2]}');`]),
      ]);
    },
    computedConst: () => {
      const l = lib('config');
      l.decl('FILES', `const FILES = ${arr(names.map(q))}.map((n) => './mods/' + n);`, true);
      h.fn('load', 'i', `return require(${from(l, 'FILES')}[i]);`);
      add('load', [() => ['0'], () => ['1'], () => ['2']]);
    },
    envConst: () => {
      const l = lib('config');
      l.decl('DIR', "const DIR = process.env.FLOW_K0 === '1' ? './other' : './mods';", true);
      h.fn('load', 'name', `return require(${from(l, 'DIR')} + '/' + name);`);
      add('load', keys);
    },
    staticMethod: () => {
      const l = lib('paths');
      l.decl('Paths', "class Paths {\n  static dir = './mods';\n  static of(name) {\n    return Paths.dir + '/' + name;\n  }\n}", true);
      h.fn('load', 'name', `return require(${from(l, 'Paths')}.of(name));`);
      add('load', keys);
    },
    objMethod: () => {
      const l = lib('paths');
      l.decl('paths', "const paths = {\n  dir: './mods',\n  of(name) {\n    return this.dir + '/' + name;\n  },\n};", true);
      h.fn('load', 'name', `return require(${from(l, 'paths')}.of(name));`);
      add('load', keys);
    },
    curried: () => {
      const l = lib('paths');
      l.decl('make', "const make = (dir) => (name) => dir + '/' + name;", true);
      h.items.push(`const of = ${from(l, 'make')}('./mods');`);
      h.fn('load', 'name', 'return require(of(name));');
      add('load', keys);
    },
    json: () => {
      p.file(`${d}/config.json`, '{ "dir": "./mods", "list": ["./mods/z"] }\n');
      h.items.push("const cfg = require('./config.json');");
      h.fn('load', 'name', p.pick(["return require(cfg.dir + '/' + name);", "return [require(cfg.dir + '/' + name), ...cfg.list.map((f) => require(f))];"]));
      add('load', keys);
    },
    defaultExport: () => {
      const l = lib('config', 'esm');
      l.items.push("export default { dir: './mods' };");
      h.fn('load', 'name', `return require(${h.imp(l.file, 'default')}.dir + '/' + name);`);
      add('load', keys);
    },
    throughBarrel: () => {
      const l = lib('config', 'esm');
      l.decl('DIR', "const DIR = './mods';", true);
      p.file(`${d}/consts.js`, "export { DIR } from './config';\nexport * from './config';\n");
      h.fn('load', 'name', `return require(${h.imp(`${d}/consts.js`, 'DIR')} + '/' + name);`);
      add('load', keys);
    },
  }, ['wholeLet', 'wholeGetter', 'wholeConfig', 'wholePick', 'wholeList', 'pickAbs', 'constantFile', 'listPush', 'computedConst']);
  return out;
}

// `mutate`: the callee, something it calls, or the caller changes the argument before the
// load.
function kMutate(p) {
  const d = p.area('mut');
  const names = p.names();
  const [a, b, c] = names;
  p.targets(`${d}/mods`, [...names, ...names.map((n) => `${n}2`)]);
  const h = p.mod(`${d}/helper.js`);
  const out = [];
  const add = (name, variants, extra = {}) => out.push({ mod: h, name, variants, ...extra });
  const objs = (...xs) => xs.map((x) => () => [x]);
  // The caller passes the whole specifier, or a name below a directory of the helper.
  const whole = p.chance(0.6);
  const L = (x) => (whole ? x : `'./mods/' + ${x}`);
  const V = (n) => q(whole ? `./mods/${n}` : n);
  const byName = `return require(${L('opts.name')});`;
  const plain = () => {
    h.fn('load', 'opts', byName);
  };
  draw(p, {
    set: () => {
      h.fn('load', 'opts', [`opts.name = ${V(b)};`, byName]);
      add('load', objs(`{ name: ${V(a)} }`, '{}'));
    },
    setIf: () => {
      h.fn('load', 'opts', [`if (!opts.name) opts.name = ${V(c)};`, byName]);
      add('load', objs('{}', `{ name: ${V(a)} }`));
    },
    defaults: () => {
      const s = p.mod(`${d}/defaults.js`);
      s.fn('applyDefaults', 'opts', p.pick([`opts.name = opts.name || ${V(c)};`, `opts.name = ${V(c)};`, `Object.assign(opts, { name: ${V(c)} });`, `if (opts.name === ${V(a)}) opts.name = ${V(b)};`, "opts.name += '2';"]));
      h.fn('load', 'opts', [`${h.imp(s.file, 'applyDefaults')}(opts);`, byName]);
      add('load', objs(`{ name: ${V(a)} }`, `{ name: ${V(b)} }`));
    },
    defaultsReturned: () => {
      const s = p.mod(`${d}/defaults.js`);
      s.fn('withDefaults', 'opts', p.pick([`return { name: ${V(c)}, ...opts };`, `return Object.assign({ name: ${V(c)} }, opts);`, `return { ...opts, name: opts.name || ${V(c)} };`]));
      h.fn('load', 'opts', [`const o = ${h.imp(s.file, 'withDefaults')}(opts);`, `return require(${L('o.name')});`]);
      add('load', objs('{}', `{ name: ${V(a)} }`));
    },
    assign: () => {
      h.fn('load', 'opts', [`Object.assign(opts, { name: ${V(b)} });`, byName]);
      add('load', objs(`{ name: ${V(a)} }`, '{}'));
    },
    assignDefaults: () => {
      h.fn('load', 'opts', [p.pick([`const o = Object.assign({ name: ${V(c)} }, opts);`, `const o = { name: ${V(c)}, ...opts };`]), `return require(${L('o.name')});`]);
      add('load', objs('{}', `{ name: ${V(a)} }`));
    },
    push: () => {
      h.fn('load', 'list', [p.pick([`list.push(${V(c)});`, `list.unshift(${V(c)});`, `list.splice(0, 1, ${V(b)});`, `list[0] = ${V(b)};`]), `return list.map((n) => require(${L('n')}));`]);
      add('load', objs(`[${V(a)}]`, `[${V(a)}, ${V(a)}]`));
    },
    reassign: () => {
      h.fn('load', 'name', [p.pick(["name = name + '2';", "name += '2';", `if (name === ${V(a)}) name = ${V(c)};`, `name = { ${V(a)}: ${V(b)}, ${V(b)}: ${V(c)}, ${V(c)}: ${V(a)} }[name];`, 'name = `${name}2`;']), `return require(${L('name')});`]);
      add('load', one([a, b, c], V));
    },
    swap: () => {
      h.fn('load', 'opts', ['const t = opts.first;', 'opts.first = opts.second;', 'opts.second = t;', `return require(${L('opts.first')});`]);
      add('load', objs(`{ first: ${V(a)}, second: ${V(b)} }`, `{ first: ${V(b)}, second: ${V(c)} }`));
    },
    remove: () => {
      h.fn('load', 'opts', ['delete opts.name;', `return require(${L(`(opts.name || ${V(c)})`)});`]);
      add('load', objs(`{ name: ${V(a)} }`));
    },
    nested: () => {
      h.fn('load', 'opts', [`opts.target.name = ${V(b)};`, `return require(${L('opts.target.name')});`]);
      add('load', objs(`{ target: { name: ${V(a)} } }`));
    },
    alias: () => {
      h.fn('load', 'opts', ['const o = opts;', `o.name = ${V(c)};`, byName]);
      add('load', objs(`{ name: ${V(a)} }`));
    },
    callerSet: () => {
      plain();
      add('load', [
        (m) => {
          const o = m.local('o');
          return withPre([o], [`const ${o} = { name: ${V(a)} };`, `${o}.name = ${V(b)};`]);
        },
        (m) => {
          const o = m.local('o');
          const o2 = m.local('alias');
          return withPre([o], [`const ${o} = { name: ${V(a)} };`, `const ${o2} = ${o};`, `${o2}.name = ${V(c)};`]);
        },
      ]);
    },
    callerTouch: () => {
      plain();
      const s = p.mod(`${d}/touch.js`);
      s.fn('touch', 'o', p.pick([`o.name = ${V(c)};`, `Object.assign(o, { name: ${V(c)} });`, "o.name += '2';"]));
      add('load', [
        (m) => {
          const o = m.local('o');
          return withPre([o], [`const ${o} = { name: ${V(a)} };`, `${m.imp(s.file, 'touch')}(${o});`]);
        },
        () => [`{ name: ${V(b)} }`],
      ]);
    },
    callerPush: () => {
      h.fn('load', 'list', `return list.map((n) => require(${L('n')}));`);
      add('load', [
        (m) => {
          const xs = m.local('xs');
          return withPre([xs], [`const ${xs} = [${V(a)}];`, `${xs}.push(${V(b)});`]);
        },
        () => [`[${V(c)}]`],
      ]);
    },
    twoCalls: () => {
      plain();
      h.fn('prep', 'o', `o.name = ${V(b)};`);
      add('load', [
        (m) => {
          const o = m.local('o');
          return withPre([o], [`const ${o} = { name: ${V(a)} };`, `${m.imp(h.file, 'prep')}(${o});`]);
        },
        () => [`{ name: ${V(c)} }`],
      ]);
    },
    state: () => {
      h.items.push(`let current = ${V(a)};`);
      h.fn('setName', 'n', 'current = n;');
      h.fn('load', '', `return require(${L('current')});`);
      add('load', [() => [], (m) => withPre([], [`${m.imp(h.file, 'setName')}(${V(b)});`]), (m) => withPre([], [`${m.imp(h.file, 'setName')}(${V(c)});`])]);
    },
    stateObject: () => {
      h.items.push(`const state = { name: ${V(a)} };`);
      h.fn('configure', 'o', 'Object.assign(state, o);');
      h.fn('load', '', `return require(${L('state.name')});`);
      add('load', [() => [], (m) => withPre([], [`${m.imp(h.file, 'configure')}({ name: ${V(b)} });`])]);
    },
    // An array method hands the elements of its receiver to a callback: one the caller
    // supplies can change them before the helper reads them.
    cbElements: () => {
      const method = p.pick(['forEach', 'map', 'some', 'filter', 'find']);
      h.fn('load', 'list, fix', [`list.${method}(fix);`, `return require(${L('list[0].name')});`]);
      const s = p.mod(`${d}/touch.js`);
      s.fn('touch', 'o', p.pick([`o.name = ${V(c)};`, `Object.assign(o, { name: ${V(c)} });`]));
      add('load', [
        () => [`[{ name: ${V(a)} }]`, `(it) => { it.name = ${V(b)}; }`],
        () => [`[{ name: ${V(a)} }]`, '() => {}'],
        (m) => [`[{ name: ${V(a)} }]`, m.imp(s.file, 'touch')],
      ]);
    },
    // The elements are new objects that hold what the caller passed.
    cbWrapped: () => {
      const wrap = p.pick(['[{ inner: opts }]', '[[opts]].map((x) => ({ inner: x[0] }))', '[{ inner: opts }, { inner: opts }]']);
      h.fn('load', 'opts, fix', [`${wrap}.forEach(fix);`, byName]);
      const s = p.mod(`${d}/touch.js`);
      s.fn('touch', 'w', `w.inner.name = ${V(c)};`);
      add('load', [
        () => [`{ name: ${V(a)} }`, `(w) => { w.inner.name = ${V(b)}; }`],
        () => [`{ name: ${V(a)} }`, '(w) => w.inner.name'],
        (m) => [`{ name: ${V(a)} }`, m.imp(s.file, 'touch')],
      ]);
    },
    // The callback is the result of calling what the caller passed (a merger factory).
    cbCurried: () => {
      h.fn('load', 'opts, make', ['[{ handler: opts }].map(make(opts));', byName]);
      const s = p.mod(`${d}/mergers.js`);
      s.fn('keep', 'next', 'return (current) => { current.seen = next.name; return current; };');
      s.fn('through', 'next', `return (current) => { current.handler.name = ${V(c)}; return current; };`);
      s.fn('direct', 'next', `return (current) => { next.name = ${V(b)}; return current; };`);
      add('load', [
        (m) => [`{ name: ${V(a)} }`, m.imp(s.file, 'keep')],
        (m) => [`{ name: ${V(a)} }`, m.imp(s.file, 'through')],
        (m) => [`{ name: ${V(a)} }`, m.imp(s.file, 'direct')],
        () => [`{ name: ${V(a)} }`, `(next) => (current) => { current.handler.name = ${V(b)}; return current; }`],
      ]);
    },
    registered: () => {
      h.items.push('const registered = [];');
      h.fn('register', 'file', 'registered.push(file);');
      h.fn('loadAll', '', 'return registered.map((f) => require(f));');
      add('loadAll', names.map((n) => (m) => withPre([], [`${m.imp(h.file, 'register')}(${q(`./mods/${n}`)});`])));
    },
  });
  return out;
}

// `recur`: a recursive walk over a nested description.
function kRecur(p) {
  const d = p.area('rec');
  const hs = ['h1', 'h2', 'h3', 'h4', 'h5'];
  p.targets(`${d}/handlers`, hs);
  const h = p.mod(`${d}/helper.js`);
  const H = (n) => q(`./handlers/${n}`);
  const out = [];
  const guarded = "if (typeof x.handler === 'string') out.push(require(x.handler));";
  let linked = false;
  let name = 'normalize';
  draw(p, {
    everyKey: () => {
      h.fn('normalize', 'x', ["if (typeof x === 'string') return [];", 'if (Array.isArray(x)) return x.flatMap((e) => normalize(e));', "if (!x || typeof x !== 'object') return [];", 'const out = [];', guarded, 'for (const k of Object.keys(x)) out.push(...normalize(x[k]));', 'return out;']);
    },
    children: () => {
      h.fn('normalize', 'x', ['const out = [];', guarded, 'for (const r of x.routes || []) out.push(...normalize(r));', 'for (const c of x.children || []) out.push(...normalize(c));', 'return out;']);
    },
    leaves: () => {
      h.fn('normalize', 'x', [
        "if (typeof x === 'string') {",
        '  try {',
        '    return [require(x)];',
        '  } catch {',
        '    return [];',
        '  }',
        '}',
        'if (Array.isArray(x)) return x.flatMap((e) => normalize(e));',
        "if (x && typeof x === 'object') return Object.values(x).flatMap((e) => normalize(e));",
        'return [];',
      ]);
    },
    visitor: () => {
      const w = p.mod(`${d}/walk.js`);
      w.fn('walk', 'x, visit', ['if (Array.isArray(x)) {', '  for (const e of x) walk(e, visit);', "} else if (x && typeof x === 'object') {", '  visit(x);', '  for (const k of Object.keys(x)) walk(x[k], visit);', '}']);
      h.fn('normalize', 'spec', ['const out = [];', `${h.imp(w.file, 'walk')}(spec, (node) => {`, "  if (typeof node.handler === 'string') out.push(require(node.handler));", '});', 'return out;']);
    },
    mutual: () => {
      // Two files that call each other.
      h.style = 'esm';
      h.exportForm = 'inline';
      const e = p.mod(`${d}/each.js`, 'esm');
      e.fn('each', 'list', `for (const x of list || []) ${e.imp(h.file, 'normalize', 'named')}(x);`, { form: 'decl' });
      h.fn('normalize', 'x', ["if (typeof x.handler === 'string') require(x.handler);", `${h.imp(e.file, 'each', 'named')}(x.routes);`, `${h.imp(e.file, 'each', 'named')}(x.children);`], { form: 'decl' });
    },
    depth: () => {
      h.fn('normalize', 'x, depth = 0', ['if (!x || depth > 6) return [];', 'if (Array.isArray(x)) return x.flatMap((e) => normalize(e, depth + 1));', "if (typeof x !== 'object') return [];", 'const out = [];', guarded, 'for (const k of Object.keys(x)) out.push(...normalize(x[k], depth + 1));', 'return out;']);
    },
    stack: () => {
      h.fn('normalize', 'spec', ['const out = [];', 'const stack = [spec];', 'while (stack.length) {', '  const x = stack.pop();', '  if (!x) continue;', "  if (typeof x.handler === 'string') out.push(require(x.handler));", '  stack.push(...(x.routes || []), ...(x.children || []));', '}', 'return out;']);
    },
    list: () => {
      linked = true;
      name = 'visit';
      h.fn('visit', 'node', p.pick([['if (!node) return;', 'require(node.handler);', 'visit(node.next);'], ['for (let n = node; n; n = n.next) require(n.handler);']]));
    },
    importing: () => {
      h.fn('normalize', 'x', ['const out = [];', "if (typeof x.handler === 'string') out.push(import(x.handler + '.js'));", 'for (const c of x.children || []) out.push(...normalize(c));', 'for (const r of x.routes || []) out.push(...normalize(r));', 'return out;']);
    },
  });
  const route = (n, extra = '') => `{ handler: ${H(n)}, method: 'GET', path: '/${n}'${extra} }`;
  const literals = linked
    ? [`{ handler: ${H('h1')}, next: { handler: ${H('h2')}, next: null } }`, `{ handler: ${H('h3')}, next: null }`, `{ handler: ${H('h4')}, next: { handler: ${H('h5')}, next: { handler: ${H('h1')}, next: null } } }`]
    : [
        `{ routes: [${route('h1', `, children: [${route('h2')}]`)}] }`,
        `{ routes: [${route('h3')}, ${route('h4', `, children: [${route('h5', `, children: [${route('h1')}]`)}]`)}] }`,
        `{ handler: ${H('h2')}, children: [] }`,
        route('h5'),
      ];
  const variants = literals.map((l) => () => [l]);
  // The description comes from a module of its own, or from a function.
  const specFile = `${d}/spec.js`;
  const s = p.mod(specFile);
  s.decl('spec', `const spec = ${literals[1]};`, true);
  variants.push((m) => [m.imp(specFile, 'spec')]);
  if (!linked) {
    s.fn('make', 'handler', "return { routes: [{ handler, method: 'POST', path: '/made' }] };");
    variants.push((m) => [`${m.imp(specFile, 'make')}(${H('h3')})`]);
    variants.push((m) => {
      const root = m.local('root');
      const child = m.local('child');
      return withPre([root], [`const ${child} = ${route('h4')};`, `const ${root} = { handler: ${H('h2')}, children: [${child}] };`, `${root}.children.push(${route('h5')});`]);
    });
  }
  out.push({ mod: h, name, variants });
  return out;
}

// `follow`: load a module the caller chooses, then what that module lists.
function kFollow(p) {
  const d = p.area('fol');
  p.targets(`${d}/handlers`, ['h1', 'h2', 'h3', 'h4']);
  const h = p.mod(`${d}/helper.js`);
  const listing = { ra: ['h1', 'h2'], rb: ['h3'], rc: ['h2', 'h4'] };
  // How a route module names a handler: relative to the helper, or to its own file.
  const routeModules = (rel, ext = '', dir = `${d}/routes`) => {
    for (const [r, hs] of Object.entries(listing)) {
      const routes = `[${hs.map((x) => `{ handler: ${q(`${rel}/${x}${ext}`)}, method: 'GET' }`).join(', ')}]`;
      const form = p.pick(['cjs', 'dot', 'esm', 'json']);
      if (form === 'json') {
        p.file(`${dir}/${r}.json`, `{ "routes": [${hs.map((x) => `{ "handler": "${rel}/${x}${ext}" }`).join(', ')}] }\n`);
        // Loaded without an extension: `.js` wins, so there is only the JSON file.
      } else p.file(`${dir}/${r}.js`, form === 'cjs' ? `module.exports = { routes: ${routes} };\n` : form === 'dot' ? `exports.routes = ${routes};\n` : `export const routes = ${routes};\n`);
    }
  };
  const table = (base = './routes') => h.decl('T', `const T = {\n  a: '${base}/ra',\n  b: '${base}/rb',\n  c: '${base}/rc',\n};`);
  const keys = one(['a', 'b', 'c']);
  const out = [];
  const add = (name, variants, extra = {}) => out.push({ mod: h, name, variants, ...extra });
  const absRoute = (r, m) => p.pick([() => `${m.path()}.join(__dirname, ${q(m.rel(`${d}/routes`))}, ${q(r)})`, () => `require.resolve(${q(`${m.spec(`${d}/routes/${r}.js`, false)}`)})`])();
  draw(p, {
    table: () => {
      // The route modules can be named through an alias (the handlers stay relative to
      // the helper).
      const shared = p.shared(d);
      routeModules('./handlers', '', shared ? `${shared.dir}/routes` : undefined);
      table(shared ? `${shared.spec}/routes` : undefined);
      h.fn('mount', 'key', ['const m = require(T[key]);', p.pick(['for (const r of m.routes) require(r.handler);', 'm.routes.forEach((r) => require(r.handler));', 'm.routes.map((r) => r.handler).forEach((f) => require(f));']), 'return m;']);
      add('mount', keys);
    },
    tableRelative: () => {
      routeModules('../handlers');
      table();
      h.fn('mount', 'key', ['const file = require.resolve(T[key]);', 'const m = require(file);', `for (const r of m.routes) require(${h.path()}.join(${h.path()}.dirname(file), r.handler));`, 'return m;']);
      add('mount', keys);
    },
    param: () => {
      routeModules('../handlers');
      h.fn('mount', 'routeFile', ['const m = require(routeFile);', `for (const r of m.routes) require(${h.path()}.join(${h.path()}.dirname(routeFile), r.handler));`, 'return m;']);
      add('mount', one(['ra', 'rb', 'rc'], absRoute));
    },
    paramHelperRelative: () => {
      routeModules('./handlers');
      h.fn('mount', 'routeFile', ['const m = require(routeFile);', 'for (const r of m.routes) require(r.handler);', 'return m;']);
      add('mount', [...one(['ra', 'rb', 'rc'], absRoute), ...one(['ra', 'rb'], (r) => q(`./routes/${r}`))]);
    },
    name: () => {
      routeModules('./handlers');
      h.fn('mount', 'name', ["const m = require('./routes/' + name);", 'for (const r of m.routes) require(r.handler);', 'return m;']);
      add('mount', one(['ra', 'rb', 'rc']));
    },
    cwd: () => {
      // Every package has its own routes: the helper loads the ones of the package the
      // test runs in.
      for (const pkg of p.pkgs) {
        p.file(`pkgs/${pkg}/src/routes.js`, `module.exports = { routes: [{ handler: './handlers/${pkg}x' }, { handler: './handlers/${pkg}y' }] };\n`);
        p.targets(`pkgs/${pkg}/src/handlers`, [`${pkg}x`, `${pkg}y`]);
      }
      h.fn(
        'mount',
        '',
        p.pick([
          [`const m = require(${h.path()}.join(process.cwd(), 'src/routes'));`, `for (const r of m.routes) require(${h.path()}.join(process.cwd(), 'src', r.handler));`, 'return m;'],
          ["const m = require(process.cwd() + '/src/routes');", "for (const r of m.routes) require(process.cwd() + '/src/' + r.handler);", 'return m;'],
          [`const file = ${h.path()}.resolve('src/routes.js');`, 'const m = require(file);', `for (const r of m.routes) require(${h.path()}.resolve(${h.path()}.dirname(file), r.handler));`, 'return m;'],
        ]),
      );
      add('mount', [() => []]);
    },
    cwdDefault: () => {
      for (const pkg of p.pkgs) {
        p.file(`pkgs/${pkg}/src/routes.js`, `module.exports = { routes: [{ handler: './handlers/${pkg}x' }, { handler: './handlers/${pkg}y' }] };\n`);
        p.targets(`pkgs/${pkg}/src/handlers`, [`${pkg}x`, `${pkg}y`]);
      }
      h.fn('mount', 'root = process.cwd()', [`const m = require(${h.path()}.join(root, 'src/routes'));`, `for (const r of m.routes) require(${h.path()}.join(root, 'src', r.handler));`, 'return m;']);
      add('mount', [() => [], () => ['undefined'], (m) => [`${m.path()}.join(__dirname, ${q(m.rel(`pkgs/${p.pkgs[0]}`))})`]]);
    },
    strings: () => {
      for (const [r, hs] of Object.entries(listing)) p.file(`${d}/routes/${r}.js`, `module.exports = ${arr(hs.map((x) => q(`./handlers/${x}`)))};\n`);
      table();
      h.fn('mount', 'key', 'return require(T[key]).map((f) => require(f));');
      add('mount', keys);
    },
    include: () => {
      p.file(`${d}/routes/ra.js`, "module.exports = { routes: [{ handler: './handlers/h1' }], include: ['./routes/rb'] };\n");
      p.file(`${d}/routes/rb.js`, "module.exports = { routes: [{ handler: './handlers/h2' }], include: ['./routes/rc'] };\n");
      p.file(`${d}/routes/rc.js`, "module.exports = { routes: [{ handler: './handlers/h3' }, { handler: './handlers/h4' }] };\n");
      h.fn('mount', 'file', ['const m = require(file);', 'for (const r of m.routes) require(r.handler);', 'for (const f of m.include || []) mount(f);', 'return m;']);
      add('mount', one(['ra', 'rb', 'rc'], (r) => q(`./routes/${r}`)));
    },
    imported: () => {
      routeModules('./handlers', '.js');
      table();
      h.fn('mount', 'key', ["const loaded = await import(T[key] + '.js');", 'const m = loaded.routes ? loaded : loaded.default;', 'for (const r of m.routes) await import(r.handler);', 'return m;'], { form: 'async' });
      // No JSON route modules here: `import()` needs an attribute for them.
      for (const r of Object.keys(listing)) {
        if (p.raw[`${d}/routes/${r}.json`]) {
          delete p.raw[`${d}/routes/${r}.json`];
          p.file(`${d}/routes/${r}.js`, `exports.routes = [${listing[r].map((x) => `{ handler: ${q(`./handlers/${x}.js`)} }`).join(', ')}];\n`);
        }
      }
      add('mount', keys);
    },
    thunks: () => {
      for (const [r, hs] of Object.entries(listing)) p.file(`${d}/routes/${r}.js`, `module.exports = { routes: [${hs.map((x) => `{ load: () => require(${q(`../handlers/${x}`)}) }`).join(', ')}] };\n`);
      table();
      h.fn('mount', 'key', 'return require(T[key]).routes.map((r) => r.load());');
      add('mount', keys);
    },
  });
  return out;
}

// `dyncall`: a module that is only loaded through a computed path; the loader calls its
// export, which has a parameter-path load itself.
function kDyncall(p) {
  const d = p.area('dyn');
  p.targets(`${d}/plugins/mods`, ['c', 'd', 'pa', 'pb']);
  p.targets(`${d}/mods`, ['c', 'd']);
  const h = p.mod(`${d}/boot.js`);
  const out = [];
  const add = (name, variants, extra = {}) => out.push({ mod: h, name, variants, ...extra });
  // What the loader calls: `call(m, arg)`.
  let call = (m, arg) => `${m}.setup(${arg})`;
  const plugin = (name) => {
    const file = `${d}/plugins/${name}.js`;
    const load = p.pick(['return require(spec);', 'return require(spec);', `return require(require('path').join(__dirname, spec));`]);
    const form = p.pick(['dot', 'esm', 'object', 'function', 'class']);
    if (form === 'dot') p.file(file, `exports.setup = function (spec) {\n  ${load}\n};\n`);
    else if (form === 'esm') p.file(file, `export function setup(spec) {\n  ${load}\n}\n`);
    else if (form === 'object') p.file(file, `module.exports = {\n  setup(spec) {\n    ${load}\n  },\n};\n`);
    else if (form === 'function') p.file(file, `module.exports = function setup(spec) {\n  ${load}\n};\nmodule.exports.setup = module.exports;\n`);
    else p.file(file, `class Plugin {\n  constructor(spec) {\n    this.spec = spec;\n  }\n  run() {\n    const spec = this.spec;\n    ${load}\n  }\n}\nexports.setup = (spec) => new Plugin(spec).run();\n`);
  };
  plugin('pa');
  plugin('pb');
  const names = one(['pa', 'pb']);
  draw(p, {
    constant: () => {
      h.fn('boot', 'name', ["const m = require('./plugins/' + name);", `return ${call('m', "'./mods/c'")};`]);
      add('boot', names);
    },
    pass: () => {
      h.fn('boot', 'name, spec', `return ${call("require('./plugins/' + name)", 'spec')};`);
      add('boot', [() => [q('pa'), q('./mods/c')], () => [q('pb'), q('./mods/d')], () => [q('pa'), q('./mods/d')]]);
    },
    table: () => {
      h.decl('P', "const P = {\n  a: './plugins/pa',\n  b: './plugins/pb',\n};");
      h.fn('boot', 'key', ['const m = require(P[key]);', `return ${call('m', "'./mods/' + key.replace('a', 'c').replace('b', 'd')")};`]);
      add('boot', one(['a', 'b']));
    },
    perPlugin: () => {
      h.fn('boot', 'name', ["const m = require('./plugins/' + name);", `return ${call('m', "'./mods/' + name")};`]);
      add('boot', names);
    },
    imported: () => {
      h.fn('boot', 'name', `return import('./plugins/' + name + '.js').then((m) => (m.setup || m.default.setup)('./mods/c'));`);
      add('boot', names);
    },
    all: () => {
      h.fn('boot', 'spec', `return ['pa', 'pb'].map((n) => ${call("require('./plugins/' + n)", 'spec')});`);
      add('boot', one(['./mods/c', './mods/d']));
    },
    callback: () => {
      // The plugin calls back into the loader, which loads relative to its own file.
      p.raw[`${d}/plugins/pa.js`] = "exports.setup = function (api) {\n  return api.load('./mods/c');\n};\n";
      p.raw[`${d}/plugins/pb.js`] = "exports.setup = function (api) {\n  return api.load('./mods/d');\n};\n";
      h.fn('boot', 'name', ["const m = require('./plugins/' + name);", 'return m.setup({ load: (spec) => require(spec) });']);
      add('boot', names);
    },
    resolved: () => {
      h.fn('boot', 'file, spec', ['const m = require(file);', `return ${call('m', 'spec')};`]);
      add('boot', [(m) => [`${m.path()}.join(__dirname, ${q(m.rel(`${d}/plugins`))}, 'pa.js')`, q('./mods/c')], (m) => [`require.resolve(${q(m.spec(`${d}/plugins/pb.js`))})`, q('./mods/d')]]);
    },
  });
  // Sometimes a test also loads a plugin statically, without calling it.
  if (p.chance(0.2)) p.pick(p.tests).side(`${d}/plugins/pa.js`);
  return out;
}

// `ctor` where the constructor gets the whole specifier: nothing of the path is in the class.
function ctorWhole(p, d, names) {
  const h = p.mod(`${d}/loader.js`);
  const field = p.pick(['#file', '#file', 'file']);
  const decl = field === '#file' ? '  #file;\n' : '';
  const extra = p.pick(['', '', `  static create(file) {\n    return new Loader(file);\n  }\n`]);
  h.decl('Loader', `class Loader {\n${decl}  constructor(file) {\n    this.${field} = file;\n  }\n${extra}  load() {\n    return require(this.${field});\n  }\n}`, true);
  const specs = one(names, (n) => q(`./mods/${n}`));
  const out = [{ mod: h, name: 'Loader', variants: specs, invoke: extra && p.chance(0.5) ? (f, a) => `${f}.create(${a[0]}).load()` : (f, a) => `new ${N(f)}(${a[0]}).load()` }];
  if (p.chance(0.5)) {
    const s = p.mod(`${d}/sub.js`);
    const L = s.imp(h.file, 'Loader', s.esm ? 'named' : undefined);
    const form = p.pick(['fixed', 'implicit', 'through']);
    if (form === 'fixed') {
      s.decl('Sub', `class Sub extends ${L} {\n  constructor() {\n    super('./mods/${names[0]}');\n  }\n}`, true);
      out.push({ mod: s, name: 'Sub', variants: [() => []], invoke: (f) => `new ${N(f)}().load()` });
    } else if (form === 'implicit') {
      s.decl('Sub', `class Sub extends ${L} {}`, true);
      out.push({ mod: s, name: 'Sub', variants: specs, invoke: (f, a) => `new ${N(f)}(${a[0]}).load()` });
    } else {
      s.decl('Sub', `class Sub extends ${L} {\n  constructor(name) {\n    super('./mods/' + name);\n  }\n}`, true);
      out.push({ mod: s, name: 'Sub', variants: one(names), invoke: (f, a) => `new ${N(f)}(${a[0]}).load()` });
    }
  }
  if (p.chance(0.4)) {
    const s = p.mod(`${d}/factory.js`);
    s.fn('make', 'file', `return new ${N(s.imp(h.file, 'Loader'))}(file);`);
    out.push({ mod: s, name: 'make', variants: specs, invoke: (f, a) => `${f}(${a[0]}).load()` });
  }
  if (p.chance(0.3)) {
    const s = p.mod(`${d}/instance.js`);
    s.decl('loader', `const loader = new ${N(s.imp(h.file, 'Loader'))}('./mods/${names[1]}');`, true);
    out.push({ mod: s, name: 'loader', variants: [() => []], invoke: (f) => `${f}.load()` });
  }
  if (field === 'file' && p.chance(0.5)) {
    // Another file changes the field after construction.
    out.push({
      mod: h,
      name: 'Loader',
      variants: specs,
      invoke: (f, a, m) => {
        const l = m.local('loader');
        return `const ${l} = new ${N(f)}('./mods/${names[0]}');\n${l}.file = ${a[0]};\n${l}.load()`;
      },
    });
  }
  return out;
}

// `ctor`: constructor parameters kept in fields, used by a method.
function kCtor(p) {
  const d = p.area('cls');
  const names = p.names();
  p.targets(`${d}/mods`, names);
  p.targets(`${d}/other`, names);
  if (p.chance(0.4)) return ctorWhole(p, d, names);
  const h = p.mod(`${d}/loader.js`);
  const field = p.pick(['#dir', '#dir', 'dir', '_dir']);
  const decl = field === '#dir' ? '  #dir;\n' : '';
  const load = `  load(name) {\n    return require(this.${field} + '/' + name);\n  }`;
  const out = [];
  const both = (dirs = ['./mods', './other']) => dirs.flatMap((dir) => names.map((n) => () => [q(dir), q(n)]));
  let variants = both();
  let invoke = (f, a) => `new ${N(f)}(${a[0]}).load(${a[1]})`;
  let cls;
  draw(p, {
    plain: () => {
      cls = `class Loader {\n${decl}  constructor(dir) {\n    this.${field} = dir;\n  }\n${load}\n}`;
    },
    options: () => {
      cls = `class Loader {\n${decl}  constructor({ dir, ext = '' } = {}) {\n    this.${field} = dir;\n    this.ext = ext;\n  }\n  load(name) {\n    return require(this.${field} + '/' + name + this.ext);\n  }\n}`;
      variants = ['./mods', './other'].flatMap((dir) => names.map((n) => () => [`{ dir: ${q(dir)} }`, q(n)]));
    },
    dflt: () => {
      cls = `class Loader {\n${decl}  constructor(dir = './mods') {\n    this.${field} = dir;\n  }\n${load}\n}`;
      variants = [...both(), () => ['', q(names[0])], () => ['undefined', q(names[1])]];
    },
    setter: () => {
      cls = `class Loader {\n${decl}  constructor(dir) {\n    this.${field} = dir;\n  }\n  setDir(dir) {\n    this.${field} = dir;\n    return this;\n  }\n${load}\n}`;
      invoke = (f, a) => `new ${N(f)}('./mods').setDir(${a[0]}).load(${a[1]})`;
    },
    factory: () => {
      cls = `class Loader {\n${decl}  constructor(dir) {\n    this.${field} = dir;\n  }\n  static create(dir) {\n    return new Loader(dir);\n  }\n${load}\n}`;
      invoke = (f, a) => `${f}.create(${a[0]}).load(${a[1]})`;
    },
    getter: () => {
      cls = `class Loader {\n${decl}  constructor(dir) {\n    this.${field} = dir;\n  }\n  get where() {\n    return this.${field};\n  }\n  load(name) {\n    return require(this.where + '/' + name);\n  }\n}`;
    },
    joined: () => {
      cls = `class Loader {\n${decl}  constructor(root) {\n    this.${field} = root;\n  }\n  load(name) {\n    return require(${h.path()}.join(this.${field}, name));\n  }\n}`;
      variants = ['mods', 'other'].flatMap((dir) => names.map((n) => (m) => [`${m.path()}.join(__dirname, ${q(m.rel(`${d}/${dir}`))})`, q(n)]));
    },
    local: () => {
      cls = `class Loader {\n${decl}  constructor(dir) {\n    this.${field} = dir;\n  }\n${load}\n}`;
      invoke = (f, a, m) => {
        const l = m.local('loader');
        return `const ${l} = new ${N(f)}(${a[0]});\n${l}.load(${a[1]})`;
      };
    },
    imports: () => {
      cls = `class Loader {\n${decl}  constructor(dir) {\n    this.${field} = dir;\n  }\n  load(name) {\n    return import(this.${field} + '/' + name + '.js');\n  }\n}`;
    },
  });
  h.decl('Loader', cls, true);
  out.push({ mod: h, name: 'Loader', variants, invoke });
  // (An absolute root has no meaning in another file's literal.)
  if (cls.includes('constructor(root)')) return out;
  const byName = one(names);
  // A subclass, a factory and an instance in other files.
  if (p.chance(0.5)) {
    const s = p.mod(`${d}/sub.js`);
    const L = s.imp(h.file, 'Loader', s.esm ? 'named' : undefined);
    const sup = cls.includes('{ dir, ext') ? "{ dir: './other' }" : "'./other'";
    const form = p.pick(['fixed', 'implicit', 'override', 'through']);
    if (form === 'fixed') {
      s.decl('Sub', `class Sub extends ${L} {\n  constructor() {\n    super(${sup});\n  }\n}`, true);
      out.push({ mod: s, name: 'Sub', variants: byName, invoke: (f, a) => `new ${N(f)}().load(${a[0]})` });
    } else if (form === 'implicit') {
      s.decl('Sub', `class Sub extends ${L} {}`, true);
      out.push({ mod: s, name: 'Sub', variants, invoke: (f, a) => `new ${N(f)}(${a[0]}).load(${a[1]})` });
    } else if (form === 'override') {
      s.decl('Sub', `class Sub extends ${L} {\n  constructor() {\n    super(${sup});\n  }\n  load(name) {\n    return super.load(name.replace('-x', ''));\n  }\n}`, true);
      out.push({ mod: s, name: 'Sub', variants: one(names, (n) => q(`${n}-x`)), invoke: (f, a) => `new ${N(f)}().load(${a[0]})` });
    } else {
      s.decl('Sub', `class Sub extends ${L} {\n  constructor(...args) {\n    super(...args);\n  }\n  get(name) {\n    return this.load(name);\n  }\n}`, true);
      out.push({ mod: s, name: 'Sub', variants, invoke: (f, a) => `new ${N(f)}(${a[0]}).get(${a[1]})` });
    }
  }
  if (p.chance(0.4)) {
    const s = p.mod(`${d}/factory.js`);
    const arg = cls.includes('{ dir, ext') ? '{ dir }' : 'dir';
    s.fn('make', 'dir', `return new ${N(s.imp(h.file, 'Loader'))}(${arg});`);
    out.push({ mod: s, name: 'make', variants: both(), invoke: (f, a) => `${f}(${a[0]}).load(${a[1]})` });
  }
  if (p.chance(0.3)) {
    const s = p.mod(`${d}/instance.js`);
    const arg = cls.includes('{ dir, ext') ? "{ dir: './mods' }" : "'./mods'";
    s.decl('loader', `const loader = new ${N(s.imp(h.file, 'Loader'))}(${arg});`, true);
    out.push({ mod: s, name: 'loader', variants: byName, invoke: (f, a) => `${f}.load(${a[0]})` });
  }
  if (field === 'dir' && p.chance(0.5)) {
    // Another file changes the field after construction.
    out.push({
      mod: h,
      name: 'Loader',
      variants: byName,
      invoke: (f, a, m) => {
        const l = m.local('loader');
        return `const ${l} = new ${N(f)}(${cls.includes('{ dir, ext') ? "{ dir: './mods' }" : "'./mods'"});\n${l}.dir = './other';\n${l}.load(${a[0]})`;
      },
    });
  }
  return out;
}

// `nonstr`: values at a load site that are not strings.
function kNonstr(p) {
  const d = p.area('non');
  p.targets(`${d}/mods`, ['a', 'b', 'n1', 'n2', 'n3', 'true', '0', '1']);
  p.target(`${d}/index.js`);
  const pkgDir = posix.dirname(d);
  p.target(`${pkgDir}/index.js`);
  const h = p.mod(`${d}/helper.js`);
  const guard = (stmt) => ['try {', `  ${stmt}`, '} catch {', '  return null;', '}'];
  const v = (...xs) => xs.map((x) => (typeof x === 'function' ? x : () => [x]));
  const absUrl = (m) => `require('url').pathToFileURL(${m.path()}.join(__dirname, ${q(m.rel(`${d}/mods`))}, 'a.js'))`;
  const [params, body, variants] = draw(p, {
    req: () => ['spec', guard('return require(spec);'), v('{}', '() => 1', "['./mods/a']", "['./mods/a', './mods/b']", q('./mods/a'), '42', 'null', "{ toString() { return './mods/b'; } }", "new String('./mods/b')")],
    imp: () => [
      'spec',
      'return import(spec).catch(() => null);',
      v("['./mods/a.js']", "['./mods/a.js', './mods/b.js']", "{ toString() { return './mods/b.js'; } }", "new String('./mods/a.js')", q('./mods/b.js'), '{}', '[]', (m) => [absUrl(m)], (m) => [`${absUrl(m)}.href`], "[['./mods/b.js']]"),
    ],
    first: () => ['spec', p.pick(['return require(spec[0]);', 'return require(spec.charAt(0));', 'return require(spec.slice(0, 1));', 'return require(spec[0] + spec[1]);']), v(q('./mods/a'), q('../x'), q('./mods/b'))],
    length: () => ['spec', "return require('./mods/n' + spec.length);", v(q('a'), q('ab'), "['x', 'y', 'z']", '{ length: 2 }')],
    number: () => ['n', "return require('./mods/n' + n);", v('1', '2', '1 + 2', q('1'), '[1, 2].length', "['3']")],
    index: () => ['i', "return require('./mods/' + i);", v('0', '1', q('a'), '2 - 1')],
    bool: () => ['flag', guard("return require('./mods/' + flag);"), v('true', 'false', '!0', q('a'))],
    typeofGuard: () => ['spec', ["if (typeof spec !== 'string') return null;", 'return require(spec);'], v(q('./mods/a'), '{}', "['./mods/b']", q('./mods/b'))],
    join: () => ['parts', "return require(parts.join('/'));", v("['.', 'mods', 'a']", "['./mods', 'b']", "['./mods/a']")],
    keys: () => ['o', "return Object.keys(o).map((k) => require('./mods/' + k));", v('{ a: 1 }', '{ a: 1, b: 2 }', '{}', "['x', 'y']")],
    templated: () => ['o', 'return require(`./mods/${o}`);', v("{ toString: () => 'a' }", q('b'), '1', "['a']")],
    thunk: () => ['get', 'return require(get());', v("() => './mods/a'", "() => './mods/b'", "function () { return './mods/a'; }")],
    elements: () => ['list', 'return list.map((f) => require(f));', v("['./mods/a']", "['./mods/a', './mods/b']", "new Set(['./mods/b']).size ? ['./mods/b'] : []", "Array.from(new Set(['./mods/a']))")],
    set: () => ['set', 'return [...set].map((f) => require(f));', v("new Set(['./mods/a'])", "new Set(['./mods/a', './mods/b'])", "['./mods/b']")],
    objectValues: () => ['o', 'return Object.values(o).map((f) => require(f));', v("{ x: './mods/a' }", "{ x: './mods/a', y: './mods/b' }", "['./mods/b']")],
  });
  h.fn('load', params, body);
  return [{ mod: h, name: 'load', variants }];
}

// `guard`: a load behind a test of its argument: tests for a string (only strings reach
// the load, whatever else a caller passes), and tests that only look like one.
function kGuard(p) {
  const d = p.area('grd');
  p.targets(`${d}/mods`, ['a', 'b', 'c']);
  const h = p.mod(`${d}/helper.js`);
  const s = p.mod(`${d}/is.js`);
  s.fn('isString', 'v', "return typeof v === 'string';");
  s.fn('loose', 'v', "return typeof v === 'string' || !!v;");
  s.fn('second', 'v, w', "return typeof w === 'string';");
  s.fn('not', 'v', "return typeof v !== 'string';");
  const is = (n) => h.imp(s.file, n);
  const load = 'return import(spec).catch(() => null);';
  const v = (...xs) => xs.map((x) => (typeof x === 'function' ? x : () => [x]));
  // A string, and values that load a module without being one.
  const any = () => v(q('./mods/a.js'), "['./mods/b.js']", "{ toString() { return './mods/c.js'; } }", "[['./mods/c.js']]", '{}', "new String('./mods/b.js')");
  const two = (second) => any().map((f) => (m) => [...f(m), second]);
  const [params, body, variants] = draw(p, {
    typeofIf: () => ['spec', [`if (typeof spec === 'string') ${load}`, 'return null;'], any()],
    callIf: () => ['spec', [`if (${is('isString')}(spec)) ${load}`, 'return null;'], any()],
    ternary: () => ['spec', `return ${p.pick(["typeof spec === 'string'", `${is('isString')}(spec)`])} ? import(spec).catch(() => null) : null;`, any()],
    and: () => ['spec', `return ${is('isString')}(spec) && import(spec).catch(() => null);`, any()],
    both: () => ['spec, on', [`if (on && ${is('isString')}(spec)) ${load}`, 'return null;'], two('true')],
    lazy: () => ['spec', [`if (${is('isString')}(spec)) return (() => import(spec).catch(() => null))();`, 'return null;'], any()],
    walk: () => [
      'spec',
      ['if (Array.isArray(spec)) return Promise.all(spec.map((x) => load(x)));', `if (${is('isString')}(spec)) ${load}`, 'return spec && spec.handler ? load(spec.handler) : null;'],
      v("[{ handler: './mods/a.js' }, './mods/b.js']", "{ handler: [{ handler: './mods/c.js' }] }", q('./mods/a.js'), "[['./mods/b.js']]", "{ handler: { toString() { return './mods/c.js'; } } }"),
    ],
    // ---- not a test for a string of the value that is loaded ----
    looseIf: () => ['spec', [`if (${is('loose')}(spec)) ${load}`, 'return null;'], any()],
    secondIf: () => ['spec', [`if (${is('second')}(spec, 'x')) ${load}`, 'return null;'], any()],
    notIf: () => ['spec', [`if (${is('not')}(spec)) ${load}`, 'return null;'], any()],
    elseIf: () => ['spec', [`if (${is('isString')}(spec)) return null;`, load], any()],
    orIf: () => ['spec, on', [`if (on || ${is('isString')}(spec)) ${load}`, 'return null;'], two('true')],
    reassigned: () => ['spec, alt', [`if (${is('isString')}(spec)) {`, '  spec = alt;', `  ${load}`, '}', 'return null;'], any().map((f) => (m) => [q('x'), ...f(m)])],
    otherVar: () => ['spec, o', [`if (${is('isString')}(o)) ${load}`, 'return null;'], two(q('x'))],
    after: () => ['spec', [`if (${is('isString')}(spec)) void 0;`, load], any()],
    member: () => ['opts', [`if (${is('isString')}(opts.spec)) {`, '  if (opts.swap) opts.swap();', '  return import(opts.spec).catch(() => null);', '}', 'return null;'], v("{ spec: './mods/a.js' }", "{ spec: './mods/a.js', swap() { this.spec = ['./mods/b.js']; } }")],
  });
  h.fn('load', params, body);
  return [{ mod: h, name: 'load', variants }];
}

// `bare`: package names and odd strings at a load site.
function kBare(p) {
  const d = p.area('bare');
  p.targets(`${d}/mods`, ['a', 'b', 'index']);
  p.target(`${d}/index.js`);
  const pkgDir = posix.dirname(d);
  for (const pkg of p.pkgs) {
    p.target(`pkgs/${pkg}/index.js`);
    p.target(`pkgs/${pkg}/lib/x.js`);
  }
  const own = posix.basename(pkgDir);
  const other = p.pkgs.find((x) => x !== own);
  const h = p.mod(`${d}/helper.js`);
  const guard = (stmt) => ['try {', `  ${stmt}`, '} catch {', '  return null;', '}'];
  const v = (...xs) => xs.map((x) => (typeof x === 'function' ? x : () => [q(x)]));
  const abs = [
    (m) => [`${m.path()}.join(__dirname, ${q(m.rel(`${d}/mods`))}, 'a.js')`],
    (m) => [`require.resolve(${q(m.spec(`${d}/mods/b.js`))})`],
    (m) => [`__dirname + ${q(`/${m.rel(`${d}/mods`)}/a.js`)}`],
    () => ["process.cwd() + '/index.js'"],
    (m) => [`${m.path()}.resolve('lib/x.js')`],
  ];
  const [params, body, variants] = draw(p, {
    req: () => ['spec', guard('return require(spec);'), [...v(other, `${other}/lib/x`, own, 'not-there', 'GET', '', '.', './', '..', './mods', './mods/', 'fs', 'node:path', './mods/a', `${other}/package.json`, '/x'), ...abs]],
    imp: () => ['spec', 'return import(spec).catch(() => null);', [...v(other, `${other}/lib/x.js`, 'not-there', 'GET', 'fs', 'node:path', './mods/a.js', `${other}/index.js`), ...abs]],
    sub: () => ['pkg, sub', guard("return require(pkg + '/' + sub);"), [() => [q(other), q('lib/x')], () => [q('.'), q('mods/a')], () => [q(own), q('index.js')], () => [q('nope'), q('x')], () => [q(other), q('index')]]],
    pkgOnly: () => ['pkg', guard("return require(pkg + '/lib/x');"), v(other, own, 'nope', '..')],
    scoped: () => ['name', guard("return require('p' + name);"), v('1', '2', '9', '1/lib/x')],
    resolved: () => ['spec', guard('return require(require.resolve(spec));'), v(other, `${other}/lib/x`, './mods/a', 'not-there', '.')],
  });
  h.fn('load', params, body);
  return [{ mod: h, name: 'load', variants }];
}

// `escape` on the helper's side: a method, a closure, a callback parameter.
function kMethod(p) {
  const d = p.area('esc');
  const names = p.names();
  p.targets(`${d}/mods`, names);
  const h = p.mod(`${d}/helper.js`);
  const spec = p.chance(0.7);
  const load = spec ? 'return require(name);' : "return require('./mods/' + name);";
  const variants = one(names, spec ? (n) => q(`./mods/${n}`) : q);
  const out = [];
  draw(p, {
    method: () => {
      h.decl('Loader', `class Loader {\n  load(name) {\n    ${load}\n  }\n}`, true);
      out.push({ mod: h, name: 'Loader', variants, invoke: (f, a) => `new ${N(f)}().load(${a[0]})` });
    },
    staticMethod: () => {
      h.decl('Loader', `class Loader {\n  static load(name) {\n    ${load}\n  }\n}`, true);
      out.push({ mod: h, name: 'Loader', variants, invoke: (f, a) => `${f}.load(${a[0]})` });
    },
    object: () => {
      h.decl('api', p.pick([`const api = {\n  load(name) {\n    ${load}\n  },\n};`, `const api = {\n  load: (name) => {\n    ${load}\n  },\n};`, `const api = {\n  loaders: {\n    js: function (name) {\n      ${load}\n    },\n  },\n  load(name) {\n    return this.loaders.js(name);\n  },\n};`]), true);
      out.push({ mod: h, name: 'api', variants, invoke: (f, a) => `${f}.load(${a[0]})` });
    },
    closure: () => {
      h.fn('makeLoader', '', `return (name) => {\n  ${load}\n};`);
      out.push({ mod: h, name: 'makeLoader', variants, invoke: (f, a) => `${f}()(${a[0]})` });
    },
    closureParam: () => {
      h.fn('makeLoader', 'dir', "return (name) => require('./' + dir + '/' + name);");
      out.push({ mod: h, name: 'makeLoader', variants: one(names), invoke: (f, a) => `${f}('mods')(${a[0]})` });
    },
    prototype: () => {
      h.items.push('function Loader() {}', `Loader.prototype.load = function (name) {\n  ${load}\n};`);
      h.expose('Loader');
      out.push({ mod: h, name: 'Loader', variants, invoke: (f, a) => `new ${N(f)}().load(${a[0]})` });
    },
    callback: () => {
      h.fn('run', 'cb, name', 'return require(cb(name));');
      const s = p.mod(`${d}/to-path.js`);
      s.fn('toPath', 'n', "return './mods/' + n;");
      out.push({
        mod: h,
        name: 'run',
        variants: names.flatMap((n) => [
          () => ["(n) => './mods/' + n", q(n)],
          (m) => [m.imp(s.file, 'toPath'), q(n)],
          (m) => {
            const f = m.local('where');
            m.items.push(`function ${f}(n) {\n  return './mods/' + n;\n}`);
            return [f, q(n)];
          },
        ]),
      });
    },
    withLoader: () => {
      h.fn('withLoader', 'fn', 'return fn((spec) => require(spec));');
      out.push({ mod: h, name: 'withLoader', variants: one(names, (n) => `(load) => load(${q(`./mods/${n}`)})`) });
    },
    each: () => {
      h.fn('each', 'names, cb', 'return names.map((n) => require(cb(n)));');
      out.push({ mod: h, name: 'each', variants: [() => [arr(names.slice(0, 2).map(q)), "(n) => './mods/' + n"], () => [arr([q(names[2])]), '(n) => `./mods/${n}`']] });
    },
    hooks: () => {
      h.items.push('const hooks = [];');
      h.fn('onLoad', 'cb', 'hooks.push(cb);');
      h.fn('runAll', '', 'return hooks.map((cb) => require(cb()));');
      out.push({ mod: h, name: 'runAll', variants: names.map((n) => (m) => withPre([], [`${m.imp(h.file, 'onLoad')}(() => ${q(`./mods/${n}`)});`])) });
    },
  });
  return out;
}

const KINDS = [
  ['lit', kLit],
  ['opaque', kOpaque],
  ['opts', kOpts],
  ['table', kTable],
  ['ret', kRet],
  ['mutate', kMutate],
  ['recur', kRecur],
  ['follow', kFollow],
  ['dyncall', kDyncall],
  ['ctor', kCtor],
  ['nonstr', kNonstr],
  ['guard', kGuard],
  ['bare', kBare],
  ['escape', kMethod],
];

// ---- callers ----

// A re-export barrel in front of the helper: what a caller imports instead.
function barrel(p, H) {
  const dir = H.mod.dir;
  const base = `./${posix.basename(H.mod.file, '.js')}`;
  const fresh = () => {
    const index = `${dir}/index.js`;
    return !p.has(index) && p.chance(0.5) ? index : `${dir}/barrel${p.id()}.js`;
  };
  const file = fresh();
  const self = `./${posix.basename(file, '.js')}`;
  const name = H.name;
  if (!H.mod.esm) {
    const form = p.pick(['whole', 'dot', 'assign', 'spread']);
    p.file(file, form === 'whole' ? `module.exports = require('${base}');\n` : form === 'dot' ? `exports.${name} = require('${base}').${name};\n` : form === 'assign' ? `Object.assign(exports, require('${base}'));\n` : `module.exports = { ...require('${base}') };\n`);
    return { file, name, how: 'req' };
  }
  const decoy = () => {
    // A second module with the same export name that loads something else.
    const f = `${dir}/decoy${p.id()}.js`;
    p.target(`${dir}/decoys/never.js`);
    p.file(f, `export function ${name}(x) {\n  return require('./decoys/' + (x ? 'never' : 'never'));\n}\nexport const other = 1;\n`);
    return `./${posix.basename(f, '.js')}`;
  };
  // (A wrapper function only fits a callable that is called as a plain function.)
  const form = p.pick(['star', 'star', 'alias', 'named', 'explicitWins', 'explicitWins2', 'cycle', 'namespace', 'dflt', 'importExport', 'chain', 'longChain', ...(H.invoke ? [] : ['local'])]);
  switch (form) {
    case 'star':
      p.file(file, `export * from '${base}';\n`);
      return { file, name };
    case 'named':
      p.file(file, `export { ${name} } from '${base}';\n`);
      return { file, name };
    case 'alias': {
      const as = `${name}Alias`;
      p.file(file, `export { ${name} as ${as} } from '${base}';\n`);
      return { file, name: as };
    }
    case 'explicitWins':
      p.file(file, `export * from '${decoy()}';\nexport { ${name} } from '${base}';\n`);
      return { file, name };
    case 'explicitWins2':
      p.file(file, `export { ${name} } from '${base}';\nexport * from '${decoy()}';\n`);
      return { file, name };
    case 'cycle': {
      const second = `${dir}/barrel${p.id()}.js`;
      p.file(file, `export * from './${posix.basename(second, '.js')}';\nexport * from '${base}';\n`);
      p.file(second, `export * from '${self}';\n`);
      return { file: p.chance(0.5) ? file : second, name };
    }
    case 'namespace':
      p.file(file, `export * as helpers from '${base}';\n`);
      return { file, name: 'helpers', member: name };
    case 'dflt':
      if (!H.mod.alsoDefault(name)) {
        p.file(file, `export * from '${base}';\n`);
        return { file, name };
      }
      if (p.chance(0.5)) {
        p.file(file, `export { default as ${name}Default } from '${base}';\n`);
        return { file, name: `${name}Default` };
      }
      p.file(file, `export { default } from '${base}';\n`);
      return { file, name: 'default' };
    case 'importExport':
      p.file(file, p.pick([`import { ${name} } from '${base}';\nexport { ${name} };\n`, `import * as all from '${base}';\nexport const ${name} = all.${name};\n`, `import { ${name} as inner } from '${base}';\nexport { inner as ${name} };\n`]));
      return { file, name };
    case 'longChain': {
      let prev = base;
      for (let i = 0; i < 14; i++) {
        const hop = `${dir}/hop${p.id()}.js`;
        p.file(hop, `export * from '${prev}';\n`);
        prev = `./${posix.basename(hop, '.js')}`;
      }
      p.file(file, `export * from '${prev}';\n`);
      return { file, name };
    }
    case 'chain': {
      const second = `${dir}/barrel${p.id()}.js`;
      p.file(second, `export * from '${base}';\n`);
      p.file(file, p.pick([`export * from './${posix.basename(second, '.js')}';\n`, `export { ${name} } from './${posix.basename(second, '.js')}';\n`]));
      return { file, name };
    }
    default:
      // A wrapper declared in the barrel.
      p.file(file, `import { ${name} as inner } from '${base}';\nexport function ${name}(...args) {\n  return inner(...args);\n}\n`);
      return { file, name, wrapper: true };
  }
}

const TRANSPORTS = ['direct', 'direct', 'direct', 'direct', 'direct', 'direct', 'direct', 'ns', 'ns', 'mid', 'mid', 'mid', 'midLit', 'midLit', 'midTop', 'midTop', 'barrel', 'barrel', 'barrel', 'barrel', 'escape', 'escape', 'escape', 'each', 'each', 'deepMid', 'setup'];
const ESCAPES = ['alias', 'call', 'apply', 'bind', 'reflect', 'map', 'forEach', 'registry', 'list', 'cbArg', 'returned', 'dynImport', 'requireInline', 'global', 'comma', 'objStore', 'optional', 'defaultParam', 'evalFile'];

const isSpread = (x) => x.startsWith('...');

// Whether the callable is a function of the module with exactly one plain parameter (an
// array method that calls it passes more arguments).
function isUnary(H) {
  const re = new RegExp(`(?:function ${H.name}|const ${H.name} = (?:function )?)\\(([^)]*)\\)`);
  for (const item of H.mod.items) {
    const m = re.exec(item);
    if (m) return m[1] !== '' && !/[,.={[]/.test(m[1]);
  }
  return false;
}

// One call of the callable `H` from the test `t`, with the arguments of variant `v`,
// reached the way `how` says.
function emit(p, H, t, v, how, depth = 0) {
  const plain = !H.invoke;
  const invoke = H.invoke ?? ((f, a) => `${f}(${a.join(', ')})`);
  const callFrom = (m, f) => {
    const a = v(m);
    return [...(a.pre ?? []), `${invoke(f, a, m)};`];
  };
  const direct = () => t.call(callFrom(t, t.imp(H.mod.file, H.name)));
  switch (how) {
    case 'ns': {
      const f = t.imp(H.mod.file, H.name, 'ns');
      const at = f.lastIndexOf('.');
      if (plain && at > 0 && /^\w+$/.test(f.slice(0, at)) && p.chance(0.3)) {
        const [o, k] = [f.slice(0, at), f.slice(at + 1)];
        const a = v(t);
        const key = t.local('key');
        t.call(...(a.pre ?? []), `const ${key} = ${q(k)};`, `${o}[${key}](${a.join(', ')});`);
      } else t.call(callFrom(t, f));
      return;
    }
    case 'mid': {
      // A module between the test and the helper that passes its parameters on.
      if (!plain || depth > 1) return direct();
      const mid = p.mod(`pkgs/${p.pick(p.pkgs)}/lib/mid${p.id()}.js`);
      const g = mid.imp(H.mod.file, H.name);
      const name = `via${p.id()}`;
      if (H.varargs || p.chance(0.3)) mid.fn(name, '...xs', `return ${g}(...xs);`);
      else mid.fn(name, 'x, y, z', p.pick([`return ${g}(x, y, z);`, [`const r = ${g}(x, y, z);`, 'return r;']]));
      return emit(p, { mod: mid, name, variants: [v], varargs: H.varargs }, t, v, p.pick(['direct', 'direct', 'ns', 'mid', 'barrel', p.clean ? 'direct' : 'escape']), depth + 1);
    }
    case 'midLit': {
      // A module in between that calls with its own values, in a function or at load time.
      const mid = p.mod(`pkgs/${p.pick(p.pkgs)}/lib/mid${p.id()}.js`);
      const name = `run${p.id()}`;
      const stmts = callFrom(mid, mid.imp(H.mod.file, H.name));
      mid.fn(name, '', stmts);
      t.call(`${t.imp(mid.file, name)}();`);
      return;
    }
    case 'midTop': {
      const mid = p.mod(`pkgs/${p.pick(p.pkgs)}/lib/mid${p.id()}.js`);
      mid.items.push(...callFrom(mid, mid.imp(H.mod.file, H.name)));
      t.side(mid.file);
      return;
    }
    case 'deepMid': {
      // More modules in between than the binder follows parameters through.
      if (!plain) return direct();
      const dir = `pkgs/${p.pick(p.pkgs)}/lib/deep${p.id()}`;
      let cur = { file: H.mod.file, name: H.name };
      for (let i = 0; i < 26; i++) {
        const mid = p.mod(`${dir}/m${i}.js`, 'esm');
        const g = mid.imp(cur.file, cur.name, 'named');
        mid.fn(`pass${i}`, H.varargs ? '...xs' : 'x, y, z', `return ${g}(${H.varargs ? '...xs' : 'x, y, z'});`);
        cur = { file: mid.file, name: `pass${i}` };
      }
      t.call(callFrom(t, t.imp(cur.file, cur.name)));
      return;
    }
    case 'setup': {
      // The call is in the setup file of the test's package.
      const s = p.setup(/^pkgs\/([^/]+)\//.exec(t.file)[1]);
      s.items.push(...callFrom(s, s.imp(H.mod.file, H.name)));
      return;
    }
    case 'barrel': {
      p.shapes.add('barrel');
      const b = barrel(p, H);
      let f = t.imp(b.file, b.name, b.how);
      if (b.member) f = `${f}.${b.member}`;
      t.call(callFrom(t, f));
      return;
    }
    case 'each': {
      const a = v(t);
      if (!plain || a.length !== 1 || a.pre || isSpread(a[0])) return t.call(...(a.pre ?? []), `${invoke(t.imp(H.mod.file, H.name), a, t)};`);
      // (A row that is an array is spread over the parameters.)
      const row = a[0].startsWith("'") && p.chance(0.5) ? a[0] : `[${a[0]}]`;
      t.items.push(`it.each([${row}])('each %s', (x) => {\n  ${t.imp(H.mod.file, H.name)}(x);\n});`);
      return;
    }
    case 'escape': {
      if (!plain) return direct();
      p.shapes.add('escape');
      const a = v(t);
      const pre = a.pre ?? [];
      const args = a.join(', ');
      const kind = p.pick(ESCAPES);
      const unary = a.length === 1 && !isSpread(a[0]) && !H.varargs && isUnary(H);
      const f = () => t.imp(H.mod.file, H.name);
      const mid = () => p.mod(`pkgs/${p.pick(p.pkgs)}/lib/mid${p.id()}.js`);
      switch (kind) {
        case 'alias': {
          const l = t.local('fn');
          return t.call(pre, `const ${l} = ${f()};`, `${l}(${args});`);
        }
        case 'call':
          return t.call(pre, `${f()}.call(${['null', ...a].join(', ')});`);
        case 'apply':
          return t.call(pre, `${f()}.apply(null, [${args}]);`);
        case 'bind':
          return t.call(pre, `${f()}.bind(${['null', ...a.slice(0, 1)].join(', ')})(${a.slice(1).join(', ')});`);
        case 'reflect':
          return t.call(pre, `Reflect.apply(${f()}, undefined, [${args}]);`);
        case 'map':
          return unary ? t.call(pre, `[${args}].map(${f()});`) : t.call(pre, `[[${args}]].map((xs) => ${f()}(...xs));`);
        case 'forEach':
          return unary ? t.call(pre, `[${args}].forEach(${f()});`) : t.call(pre, `[[${args}]].forEach((xs) => ${f()}(...xs));`);
        case 'registry': {
          const m = mid();
          const name = `registry${p.id()}`;
          m.decl(name, `const ${name} = { run: ${m.imp(H.mod.file, H.name)} };`, true);
          return t.call(pre, `${t.imp(m.file, name)}.run(${args});`);
        }
        case 'list': {
          const m = mid();
          const name = `fns${p.id()}`;
          m.decl(name, `const ${name} = [${m.imp(H.mod.file, H.name)}];`, true);
          return t.call(pre, `${t.imp(m.file, name)}[0](${args});`);
        }
        case 'cbArg': {
          const m = mid();
          const name = `apply${p.id()}`;
          m.fn(name, 'cb, ...xs', 'return cb(...xs);');
          return t.call(pre, `${t.imp(m.file, name)}(${[f(), ...a].join(', ')});`);
        }
        case 'returned': {
          const m = mid();
          const name = `get${p.id()}`;
          m.fn(name, '', `return ${m.imp(H.mod.file, H.name)};`);
          return t.call(pre, `${t.imp(m.file, name)}()(${args});`);
        }
        case 'dynImport':
          return t.call(pre, `import('${t.spec(H.mod.file, true)}').then((h) => h.${H.name}(${args}));`);
        case 'requireInline':
          return t.call(pre, `require('${t.spec(H.mod.file)}').${H.name}(${args});`);
        case 'global': {
          const m = mid();
          const name = `__helper${p.id()}`;
          m.items.push(`globalThis.${name} = ${m.imp(H.mod.file, H.name)};`);
          t.side(m.file);
          return t.call(pre, `globalThis.${name}(${args});`);
        }
        case 'comma':
          return t.call(pre, `(0, ${f()})(${args});`);
        case 'objStore': {
          const o = t.local('box');
          return t.call(pre, `const ${o} = { run: ${f()} };`, `${o}.run(${args});`);
        }
        case 'optional':
          return t.call(pre, `${f()}?.(${args});`);
        case 'evalFile': {
          // A module in between that cannot be analyzed.
          const m = mid();
          const name = `run${p.id()}`;
          m.items.push("if (process.env.FLOW_NEVER) eval('0');");
          m.fn(name, '...xs', `return ${m.imp(H.mod.file, H.name)}(...xs);`);
          return t.call(pre, `${t.imp(m.file, name)}(${args});`);
        }
        default:
          return t.call(pre, `((g = ${f()}) => g(${args}))();`);
      }
    }
    default:
      return direct();
  }
}

// Two variants joined by an opaque condition, when their arguments allow it.
function either(p, v1, v2) {
  return (m) => {
    const a = v1(m);
    const b = v2(m);
    if (a.length !== b.length || a.length === 0 || a.pre || b.pre || [...a, ...b].some((x) => x === '' || isSpread(x))) return a;
    // One condition for every position: the arguments of a call belong together.
    const bit = p.int(4);
    return a.map((x, i) => (x === b[i] ? x : `(process.env.FLOW_K${bit} === '1' ? ${x} : ${b[i]})`));
  };
}

function attach(p, H) {
  const count = 2 + p.int(3);
  for (let i = 0; i < count; i++) {
    const t = p.pick(p.tests);
    let v = p.pick(H.variants);
    if (p.chance(0.08)) v = either(p, v, p.pick(H.variants));
    let how = p.pick(TRANSPORTS);
    if (p.clean && (how === 'escape' || how === 'setup')) how = 'direct';
    emit(p, H, t, v, how);
  }
  // A test that loads the helper without calling it.
  if (p.chance(0.15)) p.pick(p.tests).side(H.mod.file);
}

export function generate(seed) {
  const p = new Proj(seed);
  const count = 1 + p.int(3);
  for (let i = 0; i < count; i++) {
    const [tag, make] = p.pick(KINDS);
    p.shapes.add(tag);
    for (const H of make(p)) attach(p, H);
  }
  return p.finish();
}
