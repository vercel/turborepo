// Containers changed through paths the use check does not see.
const fs = require('fs');
const path = require('path');
const fx = (...p) => path.join(__dirname, 'fx', ...p);
const table = ['t-a.json'];
const get = () => table;
get().push('t-b.json');
for (const f of table) { try { fs.readFileSync(fx(f)); } catch {} }
const s = new Set();
s.add('s-1.json').add('s-2.json');
for (const f of s) { try { fs.readFileSync(fx(f)); } catch {} }
const m = new Map();
m.set('a', 'm-1.json').set('b', 'm-2.json');
for (const [, f] of m) { try { fs.readFileSync(fx(f)); } catch {} }
const files = ['z.json', 'y.json'];
const sorted = files.sort();
sorted.push('pushed.json');
for (const f of files) { try { fs.readFileSync(fx(f)); } catch {} }
const reg = { dir: 'd1', set(k, v) { this.dir = v; } };
reg.set('x', 'd2');
try { fs.readFileSync(fx(reg.dir, 'f.json')); } catch {}
const reg2 = { dir: 'e1', add(v) { this.dir = v; } };
reg2.add('e2');
try { fs.readFileSync(fx(reg2.dir, 'f.json')); } catch {}
const util = { fill(t) { t.push('filled.json'); } };
const t2 = ['t2.json'];
util.fill(t2);
for (const f of t2) { try { fs.readFileSync(fx(f)); } catch {} }
const cfg = { list: ['c-1.json'] };
(cfg?.list).push('c-2.json');
for (const f of cfg.list) { try { fs.readFileSync(fx(f)); } catch {} }
const tagged = ['g-1.json'];
const tag = (strs, arr) => { arr.push('g-2.json'); };
tag`${tagged}`;
for (const f of tagged) { try { fs.readFileSync(fx(f)); } catch {} }
const queue = ['q-1.json'];
new Promise((resolve) => resolve(queue)).then((q) => { q.push('q-2.json'); for (const f of queue) { try { fs.readFileSync(fx(f)); } catch {} } });
const withed = ['w-1.json'];
for (const f of withed.with(0, 'w-2.json')) { try { fs.readFileSync(fx(f)); } catch {} }
for (const f of withed.toSpliced(0, 0, 'w-3.json')) { try { fs.readFileSync(fx(f)); } catch {} }
const proto = { inherited: 'p-1.json' };
const child = { __proto__: proto };
try { fs.readFileSync(fx(child.inherited)); } catch {}
const frozen = Object.freeze({ list: ['fz-1.json'] });
module.exports.frozen = frozen;
module.exports.frozen.list.push('fz-2.json');
for (const f of frozen.list) { try { fs.readFileSync(fx(f)); } catch {} }
const viaReturn = ['r-1.json'];
function expose() { return viaReturn; }
expose().push('r-2.json');
for (const f of viaReturn) { try { fs.readFileSync(fx(f)); } catch {} }
const nested = { inner: { f: 'n-1.json' } };
const alias = nested.inner;
module.exports.alias = alias;
module.exports.alias.f = 'n-2.json';
try { fs.readFileSync(fx(nested.inner.f)); } catch {}
const counted = { n: 0 };
function bump(o) { o.n++; }
bump(counted);
try { fs.readFileSync(fx('counted-' + counted.n)); } catch {}
const handed = { list: ['h-1.json'] };
const sink = [];
sink.push(handed.list);
sink[0].push('h-2.json');
for (const f of handed.list) { try { fs.readFileSync(fx(f)); } catch {} }
const shared = ['sh-1.json'];
const holder = new Map([['k', shared]]);
holder.get('k').push('sh-2.json');
for (const f of shared) { try { fs.readFileSync(fx(f)); } catch {} }
const registry = [];
const register = (f) => registry.push(f);
['rg-1.json', 'rg-2.json'].forEach(register);
for (const f of registry) { try { fs.readFileSync(fx(f)); } catch {} }
const byKey = {};
const put = (k, v) => { byKey[k] = v; };
put('a', 'bk-1.json');
try { fs.readFileSync(fx(byKey.a)); } catch {}
