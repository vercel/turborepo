// Numbers in paths: arithmetic, lengths, counters.
const fs = require('fs');
const path = require('path');
const fx = (...p) => path.join(__dirname, 'fx', ...p);
const xs = ['a', 'b'];
const p1 = path.join(__dirname, 'fx', 'data-' + xs.length + '.json');
try { fs.readFileSync(p1); } catch {}
const n = 1;
const p2 = path.join(__dirname, 'fx', 'f' + (n + 1) + '.json');
try { fs.readFileSync(p2); } catch {}
for (let i = 0; i < 3; i++) { const p = path.join(__dirname, 'fx', `chunk-${i + 1}.json`); try { fs.readFileSync(p); } catch {} }
let c = 0;
c += 1;
const p4 = path.join(__dirname, 'fx', `count-${c}.json`);
try { fs.readFileSync(p4); } catch {}
const counters = { n: 0 };
counters.n++;
const p5 = path.join(__dirname, 'fx', `o-${counters.n}.json`);
try { fs.readFileSync(p5); } catch {}
const table = ['zero.json', 'one.json', 'two.json'];
const one = 1;
const p6 = path.join(__dirname, 'fx', table[one + 1]);
try { fs.readFileSync(p6); } catch {}
const p7 = path.join(__dirname, 'fx', 'idx' + 'abc'.indexOf('c') + '.json');
try { fs.readFileSync(p7); } catch {}
const p8 = path.join(__dirname, 'fx', `len-${xs.push('z')}.json`);
try { fs.readFileSync(p8); } catch {}
const a = 2, b = 3;
const p9 = path.join(__dirname, 'fx', 'sum' + (a + b));
try { fs.readFileSync(p9); } catch {}
const m = new Map([['k', 'v']]);
const p10 = path.join(__dirname, 'fx', 'size' + m.size);
try { fs.readFileSync(p10); } catch {}
let total = 10;
for (const x of xs) total += x.length;
const p11 = path.join(__dirname, 'fx', 'total' + total);
try { fs.readFileSync(p11); } catch {}
const page = 1;
try { fs.readFileSync(fx(`page-${page + 1}.json`)); } catch {}
const versions = { 1: 'v1.json', 2: 'v2.json', 11: 'v11.json' };
try { fs.readFileSync(fx(versions[page + 1])); } catch {}
let seq = 0;
const nextName = () => `seq-${++seq}.json`;
try { fs.readFileSync(fx(nextName())); } catch {}
try { fs.readFileSync(fx(nextName())); } catch {}
