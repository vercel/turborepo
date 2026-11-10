// Guards: a test around a load bounds it (flow/guard.rs). The values are decoded at run time,
// so only the test can bound them; every function escapes (it is called through a list).
// g*: the test holds at the load, the loaded values must be covered. n*: shapes that only
// look like a guard; they load the values outside too, so their sites must stay open.
const fs = require('fs');
const path = require('path');
const dec = (s) => Buffer.from(s, 'hex').toString();
// ./fx/a.json ./fx/b.json ./fx/sub/c.json
const IN = [dec('2e2f66782f612e6a736f6e'), dec('2e2f66782f622e6a736f6e'), dec('2e2f66782f7375622f632e6a736f6e')];
// ./other/a.json ../fx/a.json xx./fx/a.json ./fx
const OUT = [dec('2e2f6f746865722f612e6a736f6e'), dec('2e2e2f66782f612e6a736f6e'), dec('78782e2f66782f612e6a736f6e'), dec('2e2f6678')];
const DIR = './fx/';
const LIST = ['./fx/a.json', './fx/sub/c.json'];
const SET = new Set(['./fx/b.json']);
const base = path.join(__dirname, 'fx');
const lazy = [];

function g1(name) {
  if (!name.startsWith('./fx/')) throw new Error('outside');
  return require(name);
}
function g2(name) {
  if (name.startsWith('./fx/')) {
    return require(name);
  }
  return null;
}
function g3(name) {
  return name.startsWith(DIR) ? require(name) : null;
}
function g4(name) {
  return name.startsWith('./fx/') && require(name);
}
function g5(name) {
  if (typeof name !== 'string' || !name.startsWith('./fx/')) return null;
  return require(name);
}
function g6(spec) {
  if (!spec.startsWith('./fx/')) throw new Error('outside');
  lazy.push(() => require(spec));
}
function g7(spec) {
  lazy.push(() => {
    if (!spec.startsWith('./fx/')) return null;
    return require(spec);
  });
}
function g8(name) {
  if (!/^\.\/fx\/(a|b|sub\/c)\.json$/.test(name)) throw new Error('outside');
  return require(name);
}
function g9(name) {
  if (!LIST.includes(name)) throw new Error('unknown');
  return require(name);
}
function g10(name) {
  if (SET.has(name)) return require(name);
}
function g11(name) {
  if (!name.startsWith('./fx/sub/') && !name.startsWith('./fx/a')) throw new Error('outside');
  return require(name);
}
function g12(name) {
  if (!name.startsWith('./fx/')) throw new Error('outside');
  const file = name + '';
  return require(file);
}
function g13(names) {
  for (const name of names) {
    if (!name.startsWith('./fx/')) continue;
    require(name);
  }
}
function g14(name) {
  if (!name.startsWith('./fx/')) { return null; } else { return require(name); }
}
function g15(name) {
  return !name.startsWith('./fx/') || require(name);
}
function g16(name) {
  if (!name.startsWith('./fx/')) throw new Error('outside');
  return fs.readFileSync(path.join(__dirname, name));
}
function g17(file) {
  if (!file.startsWith(base + path.sep)) throw new Error('outside');
  return require(file);
}
function g18(name) {
  if (!name.startsWith('./fx/')) throw new Error('outside');
  return function inner() { return require(name); };
}

function n1(name) {
  if (!name.startsWith('./fx/')) lazy.length;
  return require(name);
}
function n2(name) {
  if (name.startsWith('./fx/')) return null;
  return require(name);
}
function n3(name) {
  if (name.startsWith('./fx/')) { return null; } else { return require(name); }
}
function n4(name) {
  let m = null;
  try { m = require(name); } catch {}
  if (!name.startsWith('./fx/')) throw new Error('outside');
  return m;
}
function n5(name, other) {
  if (name.startsWith('./fx/') || other) return require(name);
}
function n6(name, other) {
  if (!other.startsWith('./fx/')) throw new Error('outside');
  return require(name);
}
function n7(name) {
  try { if (!name.startsWith('./fx/')) throw new Error('outside'); } catch {}
  return require(name);
}
function n8(name) {
  if (!name.startsWith('./fx/', 2)) throw new Error('outside');
  return require(name);
}
function n9(name) {
  if (!name.includes('./fx')) throw new Error('outside');
  return require(name);
}
function n10(name) {
  const early = inner();
  if (!name.startsWith('./fx/')) throw new Error('outside');
  return early;
  function inner() { try { return require(name); } catch { return null; } }
}
function n11(name, other) {
  if (!name.startsWith('./fx/')) throw new Error('outside');
  name = other;
  return require(name);
}
function n12(name) {
  if (!/fx\//.test(name)) throw new Error('outside');
  return require(name);
}
function n13(name) {
  if (!/^\.\/FX\//i.test(name)) throw new Error('outside');
  return require(name);
}
function n14(name) {
  const check = () => { if (!name.startsWith('./fx/')) throw new Error('outside'); };
  return require(name);
}

const one = [g1, g2, g3, g4, g5, g6, g7, g8, g9, g10, g11, g12, g14, g15, g16, n1, n2, n3, n4, n7, n8, n9, n10, n12, n13, n14];
for (const v of [...IN, ...OUT]) {
  for (const f of one) { try { f(v); } catch {} }
  for (const f of [n5, n6, n11]) { for (const o of [...IN, ...OUT, '']) { try { f(v, o); } catch {} } }
  try { g17(path.join(__dirname, v)); } catch {}
  try { g18(v)(); } catch {}
}
try { g13([...OUT, ...IN]); } catch {}
for (const f of lazy) { try { f(); } catch {} }
