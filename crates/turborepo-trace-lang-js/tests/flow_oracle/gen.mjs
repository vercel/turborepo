// Program generator of the value-flow oracle (see oracle.mjs): deterministic CommonJS
// programs from a seed. Every program is a list of independent segments; a segment builds
// string values through a random chain of constructs (bindings, containers, mutations
// through aliases, local functions, string and path operations, loops, classes) and hands
// them to a tracked site. The generator does not know what the values are: the oracle
// learns them by running the program.
//
// `K(i)` reads an environment bit the oracle varies between runs: an opaque condition.

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

const LEAVES = ['a.json', 'b.json', 'c.json', 'sub/c.json', 'd.txt', 'e', 'f.g.h', 'x-1.json', 'UP.json', 'q/r/s.json'];
const PRELUDE = [
  "const fs = require('fs');",
  "const path = require('path');",
  "const os = require('os');",
  "const K = (i) => process.env['FLOW_K' + i] === '1';",
  'const opt = (x) => (K(3) ? x : undefined);',
  'const it = { each: (table) => (name, fn) => table.forEach((row) => (Array.isArray(row) ? fn(...row) : fn(row))) };',
];

class Gen {
  constructor(seed) {
    this.r = rng(seed);
    this.n = 0;
    this.leafN = 0;
  }
  int(n) {
    return Math.floor(this.r() * n);
  }
  pick(xs) {
    return xs[this.int(xs.length)];
  }
  id(p = 'v') {
    return `${p}${this.n++}`;
  }
  k() {
    return `K(${this.int(4)})`;
  }
  // A fresh literal: unique per program, so a value can be traced back to its source.
  leaf() {
    const base = this.pick(LEAVES);
    return `'s${this.leafN++}-${base}'`;
  }

  // ---- string-valued expressions: { pre: statements, e: expression } ----

  expr(depth) {
    if (depth <= 0) return { pre: [], e: this.leaf() };
    const producers = [
      'lit', 'lit', 'cond', 'or', 'nullish', 'and', 'const', 'let2', 'letLate', 'varRedecl', 'plus', 'plusLoop',
      'fn', 'fn2', 'arrow', 'dflt', 'dfltRef', 'rest', 'args', 'destructParam', 'closure', 'closureMut', 'method',
      'thisMethod', 'obj', 'objComputed', 'objValues', 'objDestruct', 'objDefault', 'objSpread', 'arr', 'arrIndex',
      'arrMethod', 'arrDestruct', 'arrSpread', 'set', 'map', 'strOp', 'strOp', 'pathOp', 'num', 'iife', 'tryFinally',
      'switch', 'getter', 'hoist', 'assignExpr', 'seq', 'template', 'optChain', 'recursive', 'generator', 'tagged',
      'fromEntries', 'reduce', 'nestedFn', 'paramReassign', 'selfRef', 'entriesFind', 'json', 'manyAlts', 'labeled',
    ];
    return this[this.pick(producers)](depth - 1);
  }
  two(d) {
    const a = this.expr(d);
    const b = this.expr(d);
    return { pre: [...a.pre, ...b.pre], a: a.e, b: b.e };
  }
  lit() {
    return { pre: [], e: this.leaf() };
  }
  cond(d) {
    const t = this.two(d);
    return { pre: t.pre, e: `(${this.k()} ? ${t.a} : ${t.b})` };
  }
  or(d) {
    const t = this.two(d);
    return { pre: t.pre, e: `(opt(${t.a}) || ${t.b})` };
  }
  nullish(d) {
    const t = this.two(d);
    return { pre: t.pre, e: `(opt(${t.a}) ?? ${t.b})` };
  }
  and(d) {
    const t = this.two(d);
    return { pre: t.pre, e: `((${this.k()} && ${t.a}) || ${t.b})` };
  }
  const(d) {
    const a = this.expr(d);
    const v = this.id();
    return { pre: [...a.pre, `const ${v} = ${a.e};`], e: v };
  }
  let2(d) {
    const t = this.two(d);
    const v = this.id();
    return { pre: [...t.pre, `let ${v} = ${t.a};`, `if (${this.k()}) { ${v} = ${t.b}; }`], e: v };
  }
  letLate(d) {
    const t = this.two(d);
    const v = this.id();
    return { pre: [...t.pre, `let ${v};`, `if (${this.k()}) ${v} = ${t.a}; else ${v} = ${t.b};`], e: v };
  }
  varRedecl(d) {
    const v = this.id();
    const n = this.pick([2, 3, 10]);
    const pre = [];
    for (let i = 0; i < n; i++) {
      const a = this.expr(i === n - 1 ? d : 0);
      pre.push(...a.pre, `var ${v} = ${a.e};`);
    }
    return { pre, e: v };
  }
  plus(d) {
    const a = this.expr(d);
    const v = this.id();
    return { pre: [...a.pre, `let ${v} = ${this.leaf()};`, `${v} += ${this.pick(["'-s'", a.e, '1'])};`], e: v };
  }
  plusLoop(d) {
    const t = this.two(d);
    const v = this.id();
    return { pre: [...t.pre, `let ${v} = '';`, `for (const c of [${t.a}, ${t.b}]) ${v} += c;`], e: v };
  }
  fn(d) {
    const a = this.expr(d);
    const f = this.id('f');
    return { pre: [...a.pre, `function ${f}(p) { return p; }`], e: `${f}(${a.e})` };
  }
  fn2(d) {
    const t = this.two(d);
    const f = this.id('f');
    const u = this.id();
    return { pre: [...t.pre, `function ${f}(p) { return p; }`, `const ${u} = ${f}(${t.a});`], e: `(${this.k()} ? ${u} : ${f}(${t.b}))` };
  }
  arrow(d) {
    const a = this.expr(d);
    const f = this.id('f');
    return { pre: [...a.pre, `const ${f} = (p) => ${this.pick(['p', '{ return p; }', '(p)', 'p || p'])};`], e: `${f}(${a.e})` };
  }
  dflt(d) {
    const t = this.two(d);
    const f = this.id('f');
    return { pre: [...t.pre, `function ${f}(p = ${t.a}) { return p; }`], e: `${f}(opt(${t.b}))` };
  }
  dfltRef(d) {
    const a = this.expr(d);
    const f = this.id('f');
    return { pre: [...a.pre, `function ${f}(p, q = p + '-d') { return q; }`], e: `${f}(${a.e})` };
  }
  rest(d) {
    const t = this.two(d);
    const f = this.id('f');
    return { pre: [...t.pre, `function ${f}(...ps) { return ps[${this.k()} ? 0 : 1]; }`], e: `${f}(${t.a}, ${t.b})` };
  }
  args(d) {
    const t = this.two(d);
    const f = this.id('f');
    const body = this.pick(['return arguments[0];', 'arguments[0] = arguments[1]; return p;', 'p = arguments[1]; return arguments[0];']);
    return { pre: [...t.pre, `function ${f}(p, q) { ${body} }`], e: `${f}(${t.a}, ${t.b})` };
  }
  destructParam(d) {
    const t = this.two(d);
    const f = this.id('f');
    return { pre: [...t.pre, `function ${f}({ a, b = ${t.b} }) { return ${this.k()} ? a : b; }`], e: `${f}({ a: ${t.a} })` };
  }
  closure(d) {
    const a = this.expr(d);
    const f = this.id('f');
    return { pre: [...a.pre, `function ${f}(p) { return () => p; }`], e: `${f}(${a.e})()` };
  }
  closureMut(d) {
    const t = this.two(d);
    const v = this.id();
    const f = this.id('f');
    return { pre: [...t.pre, `let ${v} = ${t.a};`, `function ${f}(x) { ${v} = x; }`, `if (${this.k()}) ${f}(${t.b});`], e: v };
  }
  method(d) {
    const a = this.expr(d);
    const o = this.id('o');
    return { pre: [...a.pre, `const ${o} = { get(p) { return p; } };`], e: `${o}.get(${a.e})` };
  }
  thisMethod(d) {
    const t = this.two(d);
    const o = this.id('o');
    const mutate = this.pick(['', `${o}.v = ${t.b};`, `${o}.put(${t.b});`, `${o}.set('v', ${t.b});`, `${o}.add(${t.b});`]);
    return {
      pre: [...t.pre, `const ${o} = { v: ${t.a}, get() { return this.v; }, put(x) { this.v = x; }, set(k, x) { this[k] = x; }, add(x) { this.v = x; } };`, mutate ? `if (${this.k()}) ${mutate}` : ''],
      e: this.pick([`${o}.get()`, `${o}.v`]),
    };
  }
  obj(d) {
    const t = this.two(d);
    const o = this.id('o');
    return { pre: [...t.pre, `const ${o} = { k1: ${t.a}, k2: ${t.b} };`], e: `${o}.${this.pick(['k1', 'k2'])}` };
  }
  objComputed(d) {
    const t = this.two(d);
    const o = this.id('o');
    const key = this.pick([`(${this.k()} ? 'k1' : 'k2')`, `'k' + (${this.k()} ? 1 : 2)`, `['k1', 'k2'][${this.k()} ? 0 : 1]`, `String(${this.k()})`, `${this.k()}`]);
    return { pre: [...t.pre, `const ${o} = { k1: ${t.a}, k2: ${t.b}, true: ${t.a}, false: ${t.b} };`], e: `${o}[${key}]` };
  }
  objValues(d) {
    const t = this.two(d);
    const o = this.id('o');
    const read = this.pick([
      `Object.values(${o})[${this.k()} ? 0 : 1]`,
      `Object.entries(${o})[${this.k()} ? 0 : 1][1]`,
      `${o}[Object.keys(${o})[${this.k()} ? 0 : 1]]`,
      `Object.keys(${o}).map((k) => ${o}[k]).pop()`,
    ]);
    return { pre: [...t.pre, `const ${o} = { k1: ${t.a}, k2: ${t.b} };`], e: read };
  }
  objDestruct(d) {
    const t = this.two(d);
    const [x, y] = [this.id(), this.id()];
    return { pre: [...t.pre, `const { k1: ${x}, ...${y} } = { k1: ${t.a}, k2: ${t.b} };`], e: this.pick([x, `${y}.k2`]) };
  }
  objDefault(d) {
    const t = this.two(d);
    const x = this.id();
    return { pre: [...t.pre, `const { k9: ${x} = ${t.a} } = { k9: opt(${t.b}) };`], e: x };
  }
  objSpread(d) {
    const t = this.two(d);
    const [o, p] = [this.id('o'), this.id('o')];
    return { pre: [...t.pre, `const ${o} = { k1: ${t.a} };`, `const ${p} = { ...${o}, ...(${this.k()} ? { k1: ${t.b} } : {}) };`], e: `${p}.k1` };
  }
  arr(d) {
    const t = this.two(d);
    const a = this.id('a');
    return { pre: [...t.pre, `const ${a} = [${t.a}, ${t.b}];`], e: `${a}[${this.pick(['0', '1'])}]` };
  }
  arrIndex(d) {
    const t = this.two(d);
    const a = this.id('a');
    const i = this.id('i');
    const idx = this.pick([`${this.k()} ? 0 : 1`, `${a}.length - 1`, `${i} + 1`, `Number(${this.k()})`, `+${this.k()}`]);
    return { pre: [...t.pre, `const ${a} = [${t.a}, ${t.b}];`, `const ${i} = 0;`], e: `${a}[${idx}]` };
  }
  arrMethod(d) {
    const t = this.two(d);
    const a = this.id('a');
    const m = this.pick([
      '.at(-1)', '.find((x) => x)', '.map((x) => x)[1]', '.filter(Boolean)[0]', `.concat([${this.leaf()}])[2]`, '.slice(1)[0]', ".join('-')",
      '.reverse()[0]', '.pop()', '.shift()', '.flatMap((x) => [x, x])[3]', '.sort()[0]', '.toString()', '.findLast((x) => x)',
      ".reduce((acc, x) => acc + x, '')", '.entries().next().value[1]', '.values().next().value', ".map((x, i) => i + '-' + x)[1]",
      '.map((x) => x.toUpperCase())[0]', '.with(0, "w.json")[0]', '.splice(0, 1)[0]', '.join()',
    ]);
    return { pre: [...t.pre, `const ${a} = [${t.a}, ${t.b}];`], e: `${a}${m}` };
  }
  arrDestruct(d) {
    const t = this.two(d);
    const [x, y, z] = [this.id(), this.id(), this.id()];
    return { pre: [...t.pre, `const [${x}, ...${y}] = [${t.a}, ${t.b}];`, `const [, ${z} = ${this.leaf()}] = [${t.a}, opt(${t.b})];`], e: this.pick([x, `${y}[0]`, z]) };
  }
  arrSpread(d) {
    const t = this.two(d);
    const a = this.id('a');
    return { pre: [...t.pre, `const ${a} = [${t.a}];`], e: `[...${a}, ${t.b}][${this.k()} ? 0 : 1]` };
  }
  set(d) {
    const t = this.two(d);
    const s = this.id('s');
    const add = this.pick([`${s}.add(${t.b});`, `${s}.add(${t.b}).add(${this.leaf()});`, `for (const x of [${t.b}]) ${s}.add(x);`]);
    return { pre: [...t.pre, `const ${s} = new Set([${t.a}]);`, add], e: `[...${s}][${this.pick(['0', '1', `${s}.size - 1`])}]` };
  }
  map(d) {
    const t = this.two(d);
    const m = this.id('m');
    const put = this.pick([`${m}.set('j', ${t.b});`, `${m}.set('i', ${this.leaf()}).set('j', ${t.b});`, `${m}.set(${this.k()} ? 'j' : 'k', ${t.b});`]);
    return { pre: [...t.pre, `const ${m} = new Map([['k', ${t.a}]]);`, put], e: this.pick([`${m}.get('j')`, `${m}.get('k')`, `[...${m}.values()].pop()`, `Array.from(${m})[1][1]`]) };
  }
  strOp(d) {
    const a = this.expr(d);
    const op = this.pick([
      ".replace('a', 'b')", ".replace('.', '$&$&')", ".replace('s', '$$')", '.replace(/\\./, "_")', ".replaceAll('s', 'q')", '.toUpperCase()',
      '.toLowerCase()', '.trim()', '.slice(1)', '.slice(-4)', '.substring(1)', ".split('.')[0]", ".split('.').join('_')", ".concat('-c')",
      ".padStart(12, 'z')", '.repeat(2)', '.charAt(0)', '[0]', '.at(-1)', '.toString()', " + ''", '.normalize()', ".length + '.len'",
      ".indexOf('s') + '.idx'", ".startsWith('s') + '.bool'", '.valueOf()', ".split('').reverse().join('')", ".match(/[a-z]+/)[0]", '.trimEnd()',
    ]);
    const wrap = this.pick([null, null, null, 'String', 'tpl', 'enc', 'json', 'join']);
    const base = `${a.e}${op}`;
    const e =
      wrap === 'String' ? `String(${base})`
      : wrap === 'tpl' ? `\`\${${base}}\``
      : wrap === 'enc' ? `encodeURIComponent(${base})`
      : wrap === 'json' ? `JSON.parse(JSON.stringify(${base}))`
      : wrap === 'join' ? `[${base}].join('')`
      : `(${base})`;
    return { pre: a.pre, e };
  }
  pathOp(d) {
    const a = this.expr(d);
    const e = this.pick([
      `path.join('sub', ${a.e})`, `path.join(${a.e}, '..', 'sib.json')`, `path.basename(${a.e})`, `path.basename(${a.e}, '.json')`,
      `path.dirname(path.join('d1', ${a.e}))`, `path.normalize('n/../' + ${a.e})`, `path.extname(${a.e}) + '.ext'`, `path.parse(${a.e}).name`,
      `path.format({ dir: 'pf', base: ${a.e} })`, `path.relative('r', path.join('r', ${a.e}))`, `path.posix.join('pp', ${a.e})`,
      `path.join(${a.e}, '..', 'up.json')`, `path.join('a/b', '..', ${a.e})`, `path.relative(__dirname, path.resolve(__dirname, ${a.e}))`,
      `path.join(${a.e}, '.')`, `path.dirname(${a.e}) + '/dn.json'`, `path.join('x', '/', ${a.e})`,
    ]);
    return { pre: a.pre, e };
  }
  num() {
    const n = this.id('n');
    const a = this.id('a');
    const e = this.pick([
      `'n' + (${n} + 1)`, `\`n\${${n} + ${n}}\``, `'n' + ${a}.length`, `'n' + ${n}`, `'n' + (${n} - 1)`, `'n' + ${n} * 2`, `'n' + ${a}.indexOf('y')`,
      `'n' + ${a}.push('z')`, `\`n\${${n}}${n}\``, `'n' + (${this.k()} ? 1 : 2)`, `'n' + (${this.k()} + 1)`, `'n' + Number(${this.k()})`, `'n' + ${this.k()}`,
      `'n' + !${this.k()}`, `'n' + (${n} === 1)`, `'n' + ${n}++`, `'n' + ++${n}`, `'n' + (${n} += 2)`, `'n' + parseInt('7', 10)`, `'n' + 10`,
      `'n' + ${a}[${n}]`, `'n' + 1.5`, `'n' + 1e3`, `'n' + 0x10`, `'n' + -${n}`, `'n' + typeof ${n}`, `'n' + void 0`, `'n' + null`, `'n' + [1, 2]`,
    ]);
    return { pre: [`let ${n} = 1;`, `const ${a} = ['x', 'y'];`], e };
  }
  iife(d) {
    const a = this.expr(d);
    return { pre: a.pre, e: this.pick([`(function (p) { return p; })(${a.e})`, `((p) => p)(${a.e})`, `(() => { return ${a.e}; })()`]) };
  }
  tryFinally(d) {
    const t = this.two(d);
    const v = this.id();
    return { pre: [...t.pre, `let ${v} = ${this.leaf()};`, `try { ${v} = ${t.a}; if (${this.k()}) throw new Error('x'); } catch { ${v} = ${t.b}; } finally { ${v} += '.fin'; }`], e: v };
  }
  switch(d) {
    const t = this.two(d);
    const f = this.id('f');
    return { pre: [...t.pre, `function ${f}(c) { switch (c) { case true: return ${t.a}; default: { if (c === 0) return 'never'; return ${t.b}; } } }`], e: `${f}(${this.k()})` };
  }
  getter(d) {
    const t = this.two(d);
    const o = this.id('o');
    return { pre: [...t.pre, `const ${o} = { get g() { return ${this.k()} ? ${t.a} : ${t.b}; } };`], e: `${o}.g` };
  }
  hoist(d) {
    const a = this.expr(d);
    const f = this.id('f');
    const v = this.id();
    return { pre: [...a.pre, `const ${v} = ${f}();`, `function ${f}() { return ${a.e}; }`], e: v };
  }
  assignExpr(d) {
    const t = this.two(d);
    const v = this.id();
    return { pre: [...t.pre, `let ${v} = ${t.a};`], e: `(${this.k()} ? (${v} = ${t.b}) : ${v})` };
  }
  seq(d) {
    const t = this.two(d);
    return { pre: t.pre, e: `(${t.a}, ${t.b})` };
  }
  template(d) {
    const t = this.two(d);
    return { pre: t.pre, e: this.pick([`\`\${${t.a}}-\${${t.b}}\``, `\`t/\${${t.a}}\``, `\`\${${t.a}}\``, `'c' + ${t.a} + '/' + ${t.b}`]) };
  }
  optChain(d) {
    const t = this.two(d);
    const o = this.id('o');
    return { pre: [...t.pre, `const ${o} = ${this.k()} ? { k: ${t.a} } : undefined;`], e: `(${o}?.k ?? ${t.b})` };
  }
  recursive(d) {
    const a = this.expr(d);
    const f = this.id('f');
    return { pre: [...a.pre, `function ${f}(p, n) { return n > 0 ? ${f}(p + '+', n - 1) : p; }`], e: `${f}(${a.e}, ${this.pick([0, 1, 2])})` };
  }
  generator(d) {
    const t = this.two(d);
    const f = this.id('f');
    return { pre: [...t.pre, `function* ${f}() { yield ${t.a}; yield ${t.b}; }`], e: `[...${f}()][${this.k()} ? 0 : 1]` };
  }
  tagged(d) {
    const a = this.expr(d);
    const f = this.id('f');
    return { pre: [...a.pre, `const ${f} = (strs, ...vals) => strs[0] + vals.join('');`], e: this.pick([`${f}\`tg-\${${a.e}}\``, `String.raw\`raw-\${${a.e}}\``, 'String.raw`raw\\n.json`']) };
  }
  fromEntries(d) {
    const t = this.two(d);
    const o = this.id('o');
    return { pre: [...t.pre, `const ${o} = Object.fromEntries([['k1', ${t.a}], ['k2', ${t.b}]]);`], e: `${o}[${this.k()} ? 'k1' : 'k2']` };
  }
  reduce(d) {
    const t = this.two(d);
    return { pre: t.pre, e: `[${t.a}, ${t.b}].reduce((acc, x) => (${this.k()} ? acc : x))` };
  }
  nestedFn(d) {
    const t = this.two(d);
    const [f, g] = [this.id('f'), this.id('f')];
    return { pre: [...t.pre, `function ${f}(p) { function ${g}(q) { return ${this.k()} ? p : q; } return ${g}(${t.b}); }`], e: `${f}(${t.a})` };
  }
  paramReassign(d) {
    const t = this.two(d);
    const f = this.id('f');
    return { pre: [...t.pre, `function ${f}(p) { if (${this.k()}) p = ${t.b}; p = p + '.r'; return p; }`], e: `${f}(${t.a})` };
  }
  selfRef(d) {
    const a = this.expr(d);
    const v = this.id();
    return { pre: [...a.pre, `let ${v} = ${a.e};`, `for (let i = 0; i < 2; i++) ${v} = ${this.pick([`path.join('up', ${v})`, `${v} + '/x'`, `'p-' + ${v}`, `${v}.replace('s', 't')`])};`], e: v };
  }
  entriesFind(d) {
    const t = this.two(d);
    const o = this.id('o');
    return { pre: [...t.pre, `const ${o} = [{ id: 'one', file: ${t.a} }, { id: 'two', file: ${t.b} }];`], e: `${o}.find((x) => x.id === (${this.k()} ? 'one' : 'two')).file` };
  }
  json(d) {
    const a = this.expr(d);
    return { pre: a.pre, e: `JSON.parse(JSON.stringify({ f: ${a.e} })).f` };
  }
  manyAlts() {
    const n = this.pick([3, 63, 64, 65, 70, 300]);
    const a = this.id('a');
    const items = Array.from({ length: n }, (_, i) => `'m${i}.json'`).join(', ');
    return { pre: [`const ${a} = [${items}];`], e: `${a}[${this.k()} ? 0 : ${n - 1}]` };
  }
  labeled(d) {
    const t = this.two(d);
    const v = this.id();
    return { pre: [...t.pre, `let ${v} = ${t.a};`, `outer: for (const x of [1]) { for (const y of [1]) { if (${this.k()}) break outer; ${v} = ${t.b}; } }`], e: v };
  }

  // ---- sites ----

  site(e) {
    const form = this.pick([
      `fs.readFileSync(path.join(__dirname, 'fx', ${e}))`,
      `fs.readFileSync(path.join(__dirname, 'fx', ${e}))`,
      `fs.existsSync(__dirname + '/fx/' + ${e})`,
      `fs.readFileSync(\`\${__dirname}/fx/\${${e}}\`)`,
      `require('./fx/' + ${e})`,
      `require(path.join(__dirname, 'fx', ${e}))`,
      `fs.readdirSync(path.resolve(__dirname, 'fx', ${e}))`,
      `fs.statSync(path.join(__dirname, ${e}))`,
      `fs.readFileSync(path.join('fx', ${e}))`,
      `fs.promises.readFile(path.join(__dirname, 'fx', ${e})).catch(() => {})`,
    ]);
    return `try { ${form}; } catch {}`;
  }
  // A site whose argument is bound first (no inline anchored expression).
  boundSite(e) {
    const p = this.id('p');
    return [`const ${p} = path.join(__dirname, 'fx', ${e});`, `try { fs.readFileSync(${p}); } catch {}`];
  }

  // ---- segments: statements that end in one or more sites ----

  segment() {
    const kinds = ['plain', 'plain', 'plain', 'bound', 'param', 'param2', 'forOf', 'forEach', 'forIn', 'entries', 'each', 'klass', 'async', 'then', 'counter', 'container', 'container', 'container', 'container', 'exported', 'whileIdx', 'mapSite', 'guard', 'guard'];
    const body = this[`seg_${this.pick(kinds)}`]();
    // Mostly inside a function; sometimes a block at the top level (module scope, and
    // sloppy-mode function declarations in blocks).
    if (this.int(4) === 0) return ['try {', ...body.map((l) => `  ${l}`), '} catch {}'];
    const f = this.id('seg');
    return [`function ${f}() {`, ...body.map((l) => `  ${l}`), '}', `try { ${f}(); } catch {}`];
  }
  seg_plain() {
    const a = this.expr(this.pick([1, 2, 2, 3]));
    return [...a.pre.filter(Boolean), this.site(a.e)];
  }
  seg_bound() {
    const a = this.expr(this.pick([1, 2, 3]));
    return [...a.pre.filter(Boolean), ...this.boundSite(a.e)];
  }
  seg_param() {
    const a = this.expr(2);
    const f = this.id('r');
    return [...a.pre.filter(Boolean), `function ${f}(p) { ${this.site('p')} }`, `${f}(${a.e});`];
  }
  seg_param2() {
    const t = this.two(1);
    const f = this.id('r');
    const decl = this.pick([`function ${f}(p) { ${this.site('p')} }`, `const ${f} = (p) => { ${this.site('p')} };`, `const ${f} = function (p) { ${this.site('p')} };`]);
    const call2 = this.pick([`${f}(${t.b});`, `[${t.b}].forEach(${f});`, `${f}.call(null, ${t.b});`, `${f}(...[${t.b}]);`, `setTimeout(${f}, 0, ${t.b});`, `(0, ${f})(${t.b});`, `const g = ${f}; g(${t.b});`]);
    return [...t.pre.filter(Boolean), decl, `${f}(${t.a});`, call2];
  }
  seg_forOf() {
    const t = this.two(1);
    return [...t.pre.filter(Boolean), `for (const x of [${t.a}, ${t.b}]) { ${this.site('x')} }`];
  }
  seg_forEach() {
    const t = this.two(1);
    const m = this.pick(['forEach', 'map', 'filter', 'some', 'every', 'find', 'flatMap']);
    return [...t.pre.filter(Boolean), `[${t.a}, ${t.b}].${m}((x) => { ${this.site('x')} });`];
  }
  seg_forIn() {
    const t = this.two(1);
    const o = this.id('o');
    return [...t.pre.filter(Boolean), `const ${o} = { k1: ${t.a}, k2: ${t.b} };`, `for (const k in ${o}) { ${this.site(this.pick([`${o}[k]`, "k + '.json'"]))} }`];
  }
  seg_entries() {
    const t = this.two(1);
    const o = this.id('o');
    const loop = this.pick([
      `for (const [k, v] of Object.entries(${o})) { ${this.site('v')} }`,
      `Object.entries(${o}).forEach(([k, v]) => { ${this.site('v')} });`,
      `Object.keys(${o}).forEach((k) => { ${this.site(`${o}[k]`)} });`,
      `for (const v of Object.values(${o})) { ${this.site('v')} }`,
    ]);
    return [...t.pre.filter(Boolean), `const ${o} = { k1: ${t.a}, k2: ${t.b} };`, loop];
  }
  seg_each() {
    const t = this.two(1);
    const table = this.pick([`[[${t.a}, 'n'], [${t.b}, 'm']]`, `[${t.a}, ${t.b}]`, `[{ f: ${t.a} }, { f: ${t.b} }]`, `[[${t.a}], [${t.b}]]`, `[true, false]`]);
    const use = table.includes('{ f:') ? 'x.f' : table === '[true, false]' ? "'flag-' + x" : 'x';
    return [...t.pre.filter(Boolean), `it.each(${table})('t', (x, y) => { ${this.site(use)} });`];
  }
  seg_klass() {
    const t = this.two(1);
    const c = this.id('C');
    const variant = this.pick(['set', 'other', 'ctor', 'plain', 'incr', 'alias', 'static']);
    const body = {
      set: [`class ${c} { #v = ${t.a}; set(x) { this.#v = x; } run() { ${this.site('this.#v')} } }`, `const c = new ${c}();`, `if (${this.k()}) c.set(${t.b});`, 'c.run();'],
      other: [`class ${c} { #v = ${t.a}; static mk() { const c = new ${c}(); c.#v = ${t.b}; return c; } run() { ${this.site('this.#v')} } }`, `${c}.mk().run();`],
      ctor: [`class ${c} { #v; constructor(x) { this.#v = x; } run() { ${this.site('this.#v')} } }`, `new ${c}(${t.a}).run();`, `new ${c}(${t.b}).run();`],
      plain: [`class ${c} { v = ${t.a}; run() { ${this.site('this.v')} } }`, `const c = new ${c}();`, `c.v = ${t.b};`, 'c.run();'],
      incr: [`class ${c} { #n = 0; run() { this.#n++; ${this.site("'n' + this.#n")} } }`, `const c = new ${c}();`, 'c.run();', 'c.run();'],
      alias: [`class ${c} { #v = ${t.a}; put(x) { const self = this; self.#v = x; } run() { ${this.site('this.#v')} } }`, `const c = new ${c}();`, `c.put(${t.b});`, 'c.run();'],
      static: [`class ${c} { static #v = ${t.a}; static run() { ${this.site(`${c}.#v`)} } }`, `${c}.run();`],
    }[variant];
    return [...t.pre.filter(Boolean), ...body];
  }
  seg_async() {
    const a = this.expr(2);
    return [...a.pre.filter(Boolean), `(async () => { const v = await Promise.resolve(${a.e}); ${this.site('v')} })();`];
  }
  seg_then() {
    const a = this.expr(2);
    return [...a.pre.filter(Boolean), `Promise.resolve(${a.e}).then((v) => { ${this.site('v')} });`];
  }
  seg_counter() {
    const e = this.pick(["'n' + i", "'n' + (i + 1)", '`n${i}.json`', '`n${i * 2}`', "'n' + String(i)", "'n' + i.toString()", "['a', 'b', 'c'][i]", "'n' + (i + 1) + '.json'"]);
    const loop = this.pick([`for (let i = 0; i < 3; i++) { ${this.site(e)} }`, `let i = 0; while (i < 3) { ${this.site(e)} i++; }`, `let i = 0; do { ${this.site(e)} i += 1; } while (i < 3);`]);
    return [loop];
  }
  seg_whileIdx() {
    const t = this.two(1);
    const a = this.id('a');
    return [...t.pre.filter(Boolean), `const ${a} = [${t.a}, ${t.b}];`, `let i = 0;`, `while (i < ${a}.length) { ${this.site(`${a}[i]`)} i++; }`];
  }
  // A load of a parameter behind a test (flow/guard.rs), or behind something that only
  // looks like one. The values are made at run time: only the test can bound them. Two
  // lie inside the prefix, two outside; a look-alike loads those too, so its site has to
  // stay open (or cover them).
  seg_guard() {
    const [a, b, c, d] = [this.leaf(), this.leaf(), this.leaf(), this.leaf()];
    const [f, q, list] = [this.id('g'), this.id('q'), this.id('L')];
    const load = this.pick(['try { require(p); } catch {}', 'try { require(p); } catch {}', "try { fs.readFileSync(path.join(__dirname, p)); } catch {}"]);
    // A read below the file's directory is bounded to that directory whatever `p` is (a value
    // that leaves it through `..` is the documented assumption, with or without a test), so
    // the fourth value leaves the directory only for `require`.
    const away = load.includes('require(p)') ? "'../fx/'" : "'./fw/'";
    const t = "p.startsWith('./fx/')";
    const body = this.pick([
      // The test holds at the load.
      [`if (!${t}) return;`, load],
      [`if (!${t}) throw new Error('outside');`, load],
      [`if (${t}) {`, `  ${load}`, '}'],
      [`if (typeof p !== 'string' || !${t}) return;`, load],
      [`if (!${t}) { return; } else {`, `  ${load}`, '}'],
      [`if (!${list}.includes(p)) return;`, load],
      [`if (!/^\\.\\/fx\\//.test(p)) return;`, load],
      [`if (!${t}) return;`, 'const run = () => {', `  ${load}`, '};', 'run();'],
      [`if (!${t} && !p.startsWith('./fy/')) return;`, load],
      // Only looks like one.
      [`if (!${t}) ${this.k()};`, load],
      [`if (${t}) return;`, load],
      [`if (!${t} && ${this.k()}) return;`, load],
      [`try { if (!${t}) throw new Error('outside'); } catch {}`, load],
      [`if (!${t}) return;`, 'p = o;', load],
      ['const early = inner();', `if (!${t}) return early;`, 'return early;', 'function inner() {', `  ${load}`, '}'],
      [`if (!p.includes('./fx/')) return;`, load],
      [`if (!/fx\\//.test(p)) return;`, load],
    ]);
    return [
      `const ${q} = (s) => Buffer.from(s).toString();`,
      `const ${list} = ['./fx/' + ${a}];`,
      `function ${f}(p, o) {`,
      ...body.map((l) => `  ${l}`),
      '}',
      `for (const v of [${q}('./fx/' + ${a}), ${q}('./fx/' + ${b}), ${q}('./fy/' + ${c}), ${q}(${away} + ${d})]) { try { ${f}(v, ${q}('./fz/' + ${d})); } catch {} }`,
    ];
  }
  seg_mapSite() {
    const t = this.two(1);
    const a = this.id('a');
    return [...t.pre.filter(Boolean), `const ${a} = [${t.a}, ${t.b}].map((x) => path.join(__dirname, 'fx', x));`, `for (const p of ${a}) { try { fs.readFileSync(p); } catch {} }`];
  }
  seg_exported() {
    const t = this.two(1);
    const f = this.id('r');
    const how = this.pick([`module.exports.${f} = ${f};`, `exports.${f} = ${f};`, `module.exports = { ${f} };`, `globalThis.${f} = ${f};`]);
    return [...t.pre.filter(Boolean), `function ${f}(p = ${t.a}) { ${this.site('p')} }`, how, `${f}();`, `(module.exports.${f} || globalThis.${f})(${t.b});`];
  }

  // A container, a mutation through some access path, and a read of every element.
  seg_container() {
    const kind = this.pick(['array', 'array', 'object', 'object', 'set', 'map', 'nested']);
    const c = this.id('c');
    const [a, b, nv, nv2] = [this.leaf(), this.leaf(), this.leaf(), this.leaf()];
    const reader = this.id('read');
    let init;
    let muts;
    let reads;
    if (kind === 'array') {
      init = `const ${c} = [${a}, ${b}];`;
      muts = [
        `${c}.push(${nv});`, `${c}.unshift(${nv});`, `${c}[2] = ${nv};`, `${c}[${c}.length] = ${nv};`, `${c}.splice(0, 0, ${nv});`, `${c}.fill(${nv}, 0, 1);`,
        `const d = ${c}; d.push(${nv});`, `function add(t, v) { t.push(v); } add(${c}, ${nv});`, `function add() { arguments[0].push(arguments[1]); } add(${c}, ${nv});`,
        `const g = () => ${c}; g().push(${nv});`, `function g() { return ${c}; } g().push(${nv});`, `const w = { inner: ${c} }; w.inner.push(${nv});`,
        `[${c}][0].push(${nv});`, `(${this.k()} ? ${c} : ${c}).push(${nv});`, `(${c} || []).push(${nv});`, `(0, ${c}).push(${nv});`, `for (const t of [${c}]) t.push(${nv});`,
        `Array.prototype.push.call(${c}, ${nv});`, `${c}.push.apply(${c}, [${nv}]);`, `${c}['push'](${nv});`, `const m = 'push'; ${c}[m](${nv});`,
        `Object.assign(${c}, { 0: ${nv} });`, `Reflect.set(${c}, 0, ${nv});`, `Object.defineProperty(${c}, 0, { value: ${nv} });`, `[${c}[0]] = [${nv}];`,
        `({ x: ${c}[0] } = { x: ${nv} });`, `${c}?.push(${nv});`, `const d = [...${c}]; d.push(${nv});`, `const p = ${c}.push.bind(${c}); p(${nv});`,
        `${c}.length = 0; ${c}.push(${nv});`, `${c}.sort().push(${nv});`, `${c}.reverse().push(${nv});`, `const s = ${c}.sort(); s.push(${nv});`,
        `${c}.push(${nv}, ${nv2});`, `${c}.push(...[${nv}, ${nv2}]);`, `${c}.concat([${nv}]);`, `${c}.map((x) => x).push(${nv});`, `const { push } = ${c}; push.call(${c}, ${nv});`,
        `[${nv}].forEach((v) => ${c}.push(v));`, `${c}.forEach((x, i, arr) => { if (i === 0) arr.push(${nv}); });`, `new Set([${c}]).forEach((t) => t.push(${nv}));`,
        `const h = { c: ${c} }; const { c: d } = h; d.push(${nv});`, `Promise.resolve(${c}).then((t) => t.push(${nv}));`, `${c}[0] += '.x';`, `${c}.copyWithin(0, 1);`,
        `(() => ${c})().push(${nv});`, `((t) => { t.push(${nv}); })(${c});`, `const tag = (s, t) => t.push(${nv}); tag\`\${${c}}\`;`, `module.exports.c = ${c}; module.exports.c.push(${nv});`,
      ];
      reads = [`for (const x of ${c}) { ${this.site('x')} }`, `${c}.forEach((x) => { ${this.site('x')} });`, this.site(`${c}[0]`), this.site(`${c}[${c}.length - 1]`), this.site(`${c}.at(-1)`), `${c}.map((x) => x).forEach((x) => { ${this.site('x')} });`, `for (let i = 0; i < ${c}.length; i++) { ${this.site(`${c}[i]`)} }`];
    } else if (kind === 'object') {
      init = `const ${c} = { k1: ${a}, k2: ${b}, n: 0, m(v) { this.k1 = v; }, set(k, v) { this[k] = v; }, add(v) { this.k2 = v; }, push(v) { this.k1 = v; }, set sk(v) { this.k1 = v; } };`;
      muts = [
        `${c}.k3 = ${nv};`, `${c}['k3'] = ${nv};`, `${c}[${this.k()} ? 'k1' : 'k3'] = ${nv};`, `${c}.k1 = ${nv};`, `const d = ${c}; d.k3 = ${nv};`,
        `function put(t, v) { t.k3 = v; } put(${c}, ${nv});`, `const g = () => ${c}; g().k3 = ${nv};`, `function g() { return ${c}; } g().k1 = ${nv};`,
        `Object.assign(${c}, { k3: ${nv} });`, `Reflect.set(${c}, 'k1', ${nv});`, `Object.defineProperty(${c}, 'k1', { value: ${nv} });`, `${c}.k1 += '.x';`, `${c}.k9 ||= ${nv};`,
        `${c}.m(${nv});`, `${c}.set('k1', ${nv});`, `${c}.add(${nv});`, `${c}.push(${nv});`, `${c}.sk = ${nv};`, `delete ${c}.k1; ${c}.k1 = ${nv};`, `({ k1: ${c}.k1 } = { k1: ${nv} });`,
        `const w = [${c}]; w[0].k1 = ${nv};`, `for (const t of [${c}]) t.k1 = ${nv};`, `(${c}).k1 = ${nv};`, `const { m } = ${c}; m.call(${c}, ${nv});`, `Object.entries({ k1: ${nv} }).forEach(([k, v]) => { ${c}[k] = v; });`,
        `${c}.n++;`, `${c}.n += 1;`, `const o2 = { ...${c} }; o2.k1 = ${nv};`, `Object.values({ a: ${c} })[0].k1 = ${nv};`, `(() => ${c})().k1 = ${nv};`, `((t) => { t.k1 = ${nv}; })(${c});`,
        `Object.setPrototypeOf(${c}, { k7: ${nv} });`, `${c}.__proto__ = { k7: ${nv} };`, `const k = 'k1'; ${c}[k] = ${nv};`, `module.exports.c = ${c}; module.exports.c.k1 = ${nv};`,
      ];
      reads = [`for (const k in ${c}) { if (typeof ${c}[k] === 'string') { ${this.site(`${c}[k]`)} } }`, `Object.values(${c}).filter((v) => typeof v === 'string').forEach((v) => { ${this.site('v')} });`, this.site(`${c}.k1`), this.site(`${c}.k3 || ${c}.k2`), this.site(`'n' + ${c}.n`), this.site(`${c}.k7 || ${c}.k1`), this.site(`${c}.k9 || ${c}.k1`)];
    } else if (kind === 'set') {
      init = `const ${c} = new Set([${a}, ${b}]);`;
      muts = [
        `${c}.add(${nv});`, `${c}.add(${nv}).add(${nv2});`, `const d = ${c}; d.add(${nv});`, `function add(t, v) { t.add(v); } add(${c}, ${nv});`, `const g = () => ${c}; g().add(${nv});`,
        `[${nv}, ${nv2}].forEach((v) => ${c}.add(v));`, `[${nv}].forEach(${c}.add, ${c});`, `const d = new Set(${c}); d.add(${nv});`, `Set.prototype.add.call(${c}, ${nv});`, `${c}.clear(); ${c}.add(${nv});`,
        `const s2 = ${c}.add(${nv}); s2.add(${nv2});`, `(${c}).add(${nv});`, `${c}?.add(${nv});`, `for (const t of [${c}]) t.add(${nv});`,
      ];
      reads = [`for (const x of ${c}) { ${this.site('x')} }`, `${c}.forEach((x) => { ${this.site('x')} });`, `[...${c}].forEach((x) => { ${this.site('x')} });`, `Array.from(${c}).map((x) => { ${this.site('x')} });`, `for (const x of ${c}.values()) { ${this.site('x')} }`];
    } else if (kind === 'map') {
      init = `const ${c} = new Map([['k1', ${a}], ['k2', ${b}]]);`;
      muts = [
        `${c}.set('k3', ${nv});`, `${c}.set('k3', ${nv}).set('k4', ${nv2});`, `${c}.set(${this.k()} ? 'k1' : 'k3', ${nv});`, `const d = ${c}; d.set('k3', ${nv});`,
        `function put(t, v) { t.set('k3', v); } put(${c}, ${nv});`, `const g = () => ${c}; g().set('k1', ${nv});`, `Map.prototype.set.call(${c}, 'k1', ${nv});`,
        `const m2 = ${c}.set('k3', ${nv}); m2.set('k1', ${nv2});`, `[['k3', ${nv}]].forEach(([k, v]) => ${c}.set(k, v));`, `${c}.delete('k1'); ${c}.set('k1', ${nv});`, `for (const t of [${c}]) t.set('k1', ${nv});`,
      ];
      reads = [`for (const [k, v] of ${c}) { ${this.site('v')} }`, `${c}.forEach((v) => { ${this.site('v')} });`, this.site(`${c}.get('k1')`), this.site(`${c}.get('k3') || ${c}.get('k2')`), `for (const v of ${c}.values()) { ${this.site('v')} }`, `[...${c}.entries()].forEach(([k, v]) => { ${this.site('v')} });`];
    } else {
      init = `const ${c} = { list: [${a}], inner: { f: ${b} }, rows: [{ f: ${a} }] };`;
      muts = [
        `${c}.list.push(${nv});`, `${c}.inner.f = ${nv};`, `${c}.rows[0].f = ${nv};`, `const l = ${c}.list; l.push(${nv});`, `const { list } = ${c}; list.push(${nv});`, `const { inner } = ${c}; inner.f = ${nv};`,
        `function add(t, v) { t.push(v); } add(${c}.list, ${nv});`, `for (const r of ${c}.rows) r.f = ${nv};`, `${c}.rows.forEach((r) => { r.f = ${nv}; });`, `Object.assign(${c}.inner, { f: ${nv} });`,
        `${c}.list = [${nv}];`, `${c}.inner = { f: ${nv} };`, `${c}.rows.push({ f: ${nv} });`, `(${c}?.list).push(${nv});`, `${c}.list?.push(${nv});`, `Object.values(${c})[0].push(${nv});`,
        `const copy = { ...${c} }; copy.list.push(${nv});`, `const copy = [...${c}.rows]; copy[0].f = ${nv};`, `const [r0] = ${c}.rows; r0.f = ${nv};`, `${c}.rows.map((r) => r)[0].f = ${nv};`,
        `${c}.rows.find((r) => r).f = ${nv};`, `${c}.list.sort().push(${nv});`, `const w = new Map([['x', ${c}.inner]]); w.get('x').f = ${nv};`, `[${c}.inner].forEach((t) => { t.f = ${nv}; });`,
        `const g = () => ${c}.list; g().push(${nv});`, `module.exports.l = ${c}.list; module.exports.l.push(${nv});`, `JSON.parse(JSON.stringify(${c})).list.push(${nv});`,
      ];
      reads = [`for (const x of ${c}.list) { ${this.site('x')} }`, this.site(`${c}.inner.f`), `for (const r of ${c}.rows) { ${this.site('r.f')} }`, this.site(`${c}.rows[0].f`), this.site(`${c}.list[${c}.list.length - 1]`)];
    }
    const mut = this.pick(muts);
    const mut2 = this.int(3) === 0 ? this.pick(muts) : '';
    const guard = this.pick(['', '', `if (${this.k()}) `]);
    const read = this.pick(reads);
    const late = this.int(2) === 0;
    const lines = [init];
    if (late) lines.push(`function ${reader}() { ${read} }`);
    lines.push(`${guard}{ ${mut} }`);
    if (mut2) lines.push(`{ ${mut2} }`);
    lines.push(late ? `${reader}();` : read);
    return lines;
  }
}

// A module load resolved from another place (see cases/located.cjs and oracle.mjs): a
// directory, a `require` function made for it or a `resolve` call searching from it, and a
// request. `fo-dep` exists next to the program's package, next to `other/` and inside
// `fo-eve`, so the place decides which copy loads; a request that does not resolve from a
// place loads nothing. `n` makes the names unique.
function located(r, n) {
  const pick = (xs) => xs[Math.floor(r() * xs.length)];
  const k = () => `K(${Math.floor(r() * 4)})`;
  const dir = pick([
    "path.join(__dirname, '../../other')",
    "path.resolve(__dirname, '..', '..', 'other')",
    "path.join(__dirname, '..', '..') + '/other'",
    "path.dirname(require.resolve('fo-eve/package.json'))",
    "path.dirname(require.resolve('fo-eve'))",
    "path.join(require.resolve('fo-eve/package.json'), '..', 'dist')",
    '__dirname',
    "path.join(__dirname, 'fx')",
    "path.join(__dirname, K(0) ? '../../other' : 'fx')",
  ]);
  const request = pick(["'fo-dep'", "'fo-dep'", "'fo-dep/package.json'", "'./x.js'", "'./inner.js'", "'./own.cjs'", "K(1) ? 'fo-dep' : './x.js'"]);
  const [d, f, o] = [`ld${n}`, `lf${n}`, `lo${n}`];
  const forms = [
    [`const ${f} = createRequire(path.join(${d}, 'noop.js'));`, `try { ${f}(${request}); } catch {}`],
    [`const ${f} = createRequire(${d} + '/');`, `try { ${f}(${request}); } catch {}`],
    [`const ${f} = createRequire(pathToFileURL(path.join(${d}, 'noop.js')));`, `try { ${f}(${request}); } catch {}`],
    [`let ${f} = createRequire(__filename);`, `if (${k()}) ${f} = createRequire(${d} + '/');`, `try { ${f}(${request}); } catch {}`],
    [`let ${f} = createRequire(${d} + '/');`, `function ${o}() { try { return ${f}(${request}); } catch {} }`, `if (${k()}) ${f} = createRequire(__filename);`, `${o}();`],
    [`try { createRequire(path.join(${d}, 'package.json'))(${request}); } catch {}`],
    [`try { require(createRequire(${d} + '/').resolve(${request})); } catch {}`],
    [`function ${o}() { const require = createRequire(${d} + '/'); try { return require(${request}); } catch {} }`, `${o}();`],
    [`const ${f} = createRequire(path.join(${d}, 'noop.js'));`, `function ${o}(${f}) { try { return ${f}(${request}); } catch {} }`, `${o}(require);`],
    [`let ${f};`, `try { ${f} = require.resolve(${request}, { paths: [${d}] }); } catch {}`, `try { require(${f}); } catch {}`],
    [`try { require(require.resolve(${request}, { paths: [${d}, __dirname] })); } catch {}`],
    [`const ${o} = { paths: [${d}] };`, `let ${f};`, `try { ${f} = require.resolve(${request}, ${o}); } catch {}`, `try { require(${f}); } catch {}`],
    [`const ${o} = { paths: [${d}] };`, `if (${k()}) ${o}.paths = [__dirname];`, `let ${f};`, `try { ${f} = require.resolve(${request}, ${o}); } catch {}`, `try { require(${f}); } catch {}`],
    [`import(pathToFileURL(path.join(${d}, 'inner.js')).href).catch(() => {});`],
    [`import(new URL('./inner.js', pathToFileURL(${d} + '/')).href).catch(() => {});`],
    [`try { require(path.join(${d}, 'x.js')); } catch {}`],
  ];
  return [`const ${d} = ${dir};`, ...pick(forms)];
}

export function generate(seed) {
  const g = new Gen(seed);
  const lines = [...PRELUDE];
  const n = 4 + g.int(5);
  for (let i = 0; i < n; i++) lines.push(...g.segment());
  // Drawn from a stream of their own: the segments above do not depend on these forms.
  const r = rng(seed ^ 0x2545f491);
  if (r() < 0.4) {
    lines.push("const { createRequire } = require('node:module');", "const { pathToFileURL } = require('node:url');");
    const m = 1 + Math.floor(r() * 3);
    for (let i = 0; i < m; i++) lines.push(...located(r, i));
  }
  return lines.join('\n') + '\n';
}
