// Booleans, null and undefined as keys and as parts of file names.
const fs = require('fs');
const path = require('path');
const K = (i) => process.env['FLOW_K' + i] === '1';
const fx = (...p) => path.join(__dirname, 'fx', ...p);
const it = { each: (table) => (name, fn) => table.forEach((row) => (Array.isArray(row) ? fn(...row) : fn(row))) };
it.each([true, false])('t', (minify) => { try { fs.readFileSync(path.join(__dirname, 'fx', `out-${minify}.js`)); } catch {} });
const flag = K(0);
const t = { true: 'yes.json', false: 'no.json' };
const p1 = path.join(__dirname, 'fx', t[flag]);
try { fs.readFileSync(p1); } catch {}
const p2 = path.join(__dirname, 'fx', 'feature-' + flag + '.json');
try { fs.readFileSync(p2); } catch {}
const p3 = path.join(__dirname, 'fx', String(flag) + '.json');
try { fs.readFileSync(p3); } catch {}
const p4 = path.join(__dirname, 'fx', t[String(flag)]);
try { fs.readFileSync(p4); } catch {}
const p5 = path.join(__dirname, 'fx', t[flag === true]);
try { fs.readFileSync(p5); } catch {}
const u = { undefined: 'u.json', null: 'n.json' };
let key;
const p6 = path.join(__dirname, 'fx', u[key]);
try { fs.readFileSync(p6); } catch {}
const p7 = path.join(__dirname, 'fx', u[null]);
try { fs.readFileSync(p7); } catch {}
const sub = '';
const p8 = path.join(__dirname, 'fx', sub && 'nope');
try { fs.readdirSync(p8); } catch {}
const p9 = path.join(__dirname, 'fx', `${!flag}.json`);
try { fs.readFileSync(p9); } catch {}
const p10 = path.join(__dirname, 'fx', flag.toString() + '.txt');
try { fs.readFileSync(p10); } catch {}
const modes = { on: true, off: false };
for (const k in modes) { try { fs.readFileSync(fx(`mode-${modes[k]}.json`)); } catch {} }
const p11 = [__dirname, 'fx', K(1) ? 'sub' : undefined, 'c.json'].join('/');
try { fs.readFileSync(p11); } catch {}
const p12 = [__dirname, 'fx', null, 'a.json'].join('/');
try { fs.readFileSync(p12); } catch {}
const strict = true;
try { fs.readFileSync(fx(`strict-${strict}.txt`)); } catch {}
