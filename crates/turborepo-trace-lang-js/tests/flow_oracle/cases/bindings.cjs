// Redeclarations, hoisting, recursion, call shapes.
const fs = require('fs');
const path = require('path');
const K = (i) => process.env['FLOW_K' + i] === '1';
const fx = (...p) => path.join(__dirname, 'fx', ...p);
var v = 'v0.json';
var v = 'v1.json';
var v = 'v2.json';
var v = 'v3.json';
var v = 'v4.json';
var v = 'v5.json';
var v = 'v6.json';
var v = 'v7.json';
var v = 'v8.json';
var v = 'v9.json';
try { fs.readFileSync(fx(v)); } catch {}
function dup() { return 'dup1.json'; }
function dup() { return 'dup2.json'; }
try { fs.readFileSync(fx(dup())); } catch {}
function g(p) { var p = p || 'dflt.json'; try { fs.readFileSync(fx(p)); } catch {} }
g();
g('given.json');
(function rec(p) { try { fs.readFileSync(fx(p)); } catch {} if (p === 'first.json') rec('second.json'); })('first.json');
function viaApply(p) { try { fs.readFileSync(fx(p)); } catch {} }
viaApply('direct.json');
viaApply.apply(null, ['applied.json']);
function viaBind(p) { try { fs.readFileSync(fx(p)); } catch {} }
viaBind('direct2.json');
const bound = viaBind.bind(null, 'bound.json');
bound();
function viaNew(p) { try { fs.readFileSync(fx(p)); } catch {} }
viaNew('direct3.json');
new viaNew('constructed.json');
function viaTag(strs, p) { try { fs.readFileSync(fx(p || strs[0])); } catch {} }
viaTag(['direct4.json']);
viaTag`tagged.json`;
function viaArgs(p) { arguments[0] = 'rewritten.json'; try { fs.readFileSync(fx(p)); } catch {} }
viaArgs('orig.json');
function swap(a, b) { [a, b] = [b, a]; try { fs.readFileSync(fx(a)); } catch {} }
swap('left.json', 'right.json');
let late;
function setLate() { late = 'late.json'; }
function readLate() { try { fs.readFileSync(fx(late)); } catch {} }
setLate();
readLate();
let upd = 'u0';
({ upd } = { upd: 'u1' });
try { fs.readFileSync(fx(upd)); } catch {}
let arrUpd = 'au0';
[arrUpd = 'au-default'] = [];
try { fs.readFileSync(fx(arrUpd)); } catch {}
function dflt(a, b = a + '.bak') { try { fs.readFileSync(fx(b)); } catch {} }
dflt('one');
dflt('two', undefined);
dflt('three', 'explicit.json');
const handlers = { load(p) { try { fs.readFileSync(fx(p)); } catch {} } };
handlers.load('handled.json');
handlers['lo' + 'ad']('computed-call.json');
function outer() { return inner; function inner(p) { try { fs.readFileSync(fx(p)); } catch {} } }
outer()('returned-fn.json');
function once(p) { try { fs.readFileSync(fx(p)); } catch {} }
once('once.json');
globalThis.setTimeout(once, 0, 'timer.json');
function hoisted() { return target; var target = 'unreached.json'; }
try { fs.readFileSync(fx(String(hoisted()))); } catch {}
try { throw 'thrown.json'; } catch (e) { try { fs.readFileSync(fx(e)); } catch {} }
label: { var lv = 'l1.json'; if (K(0)) break label; lv = 'l2.json'; }
try { fs.readFileSync(fx(lv)); } catch {}
const self = { name: 'self.json', read() { try { fs.readFileSync(fx(this.name)); } catch {} } };
self.read();
self.read.call({ name: 'other-this.json' });
function ctor() { this.f = 'ctor.json'; }
try { fs.readFileSync(fx(new ctor().f)); } catch {}
const getterObj = { get f() { return 'getter.json'; } };
try { fs.readFileSync(fx(getterObj.f)); } catch {}
const px = new Proxy({ f: 'target.json' }, { get: () => 'proxied.json' });
try { fs.readFileSync(fx(px.f)); } catch {}
const refl = { f: 'r0.json' };
Reflect.set(refl, 'f', 'r1.json');
try { fs.readFileSync(fx(refl.f)); } catch {}
function* gen() { yield 'y1.json'; yield 'y2.json'; }
for (const y of gen()) { try { fs.readFileSync(fx(y)); } catch {} }
const [first = 'df.json', ...others] = [undefined, 'o1.json', 'o2.json'];
try { fs.readFileSync(fx(first)); } catch {}
for (const o of others) { try { fs.readFileSync(fx(o)); } catch {} }
const { a: { b: deep = 'deep-default.json' } = {} } = { a: {} };
try { fs.readFileSync(fx(deep)); } catch {}
if (K(1)) { function blockFn() { return 'block-a.json'; } } else { function blockFn() { return 'block-b.json'; } }
try { fs.readFileSync(fx(blockFn())); } catch {}
var twice = 'tw-1.json';
function shadow() { try { fs.readFileSync(fx(twice)); } catch {} var twice = 'tw-2.json'; try { fs.readFileSync(fx(twice)); } catch {} }
shadow();
const fnTable = { a: () => 'ft-a.json', b: () => 'ft-b.json' };
try { fs.readFileSync(fx(fnTable[K(2) ? 'a' : 'b']())); } catch {}
const curry = (a) => (b) => a + '-' + b;
try { fs.readFileSync(fx(curry('cu')('rry.json'))); } catch {}
const pipeline = [(s) => s + '-1', (s) => s + '-2'];
try { fs.readFileSync(fx(pipeline.reduce((acc, f) => f(acc), 'pipe'))); } catch {}
function cb(run) { run('cb-arg.json'); }
cb((p) => { try { fs.readFileSync(fx(p)); } catch {} });
function many(a, b, c) { try { fs.readFileSync(fx(c)); } catch {} }
const spreadArgs = ['m-a', 'm-b', 'm-c.json'];
many(...spreadArgs);
many('x', ...['y', 'm-d.json']);
