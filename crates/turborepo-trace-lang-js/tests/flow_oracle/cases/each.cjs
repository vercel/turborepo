// Table-driven tests.
const fs = require('fs');
const path = require('path');
const fx = (...p) => path.join(__dirname, 'fx', ...p);
const it = { each: (table) => (name, fn) => table.forEach((row) => (Array.isArray(row) ? fn(...row) : fn(row))) };
const table = [['e-a.json', 1]];
table.push(['e-b.json', 2]);
it.each(table)('t', (f) => { try { fs.readFileSync(fx(f)); } catch {} });
it.each([{ file: 'e-c.json' }, { file: 'e-d.json' }])('t', ({ file }) => { try { fs.readFileSync(fx(file)); } catch {} });
const byName = { one: 'e-e.json', two: 'e-f.json' };
it.each(Object.entries(byName))('t', (name, file) => { try { fs.readFileSync(fx(file)); } catch {} });
it.each(Object.keys(byName))('t', (name) => { try { fs.readFileSync(fx(byName[name])); } catch {} });
it.each(['e-g.json', 'e-h.json'].map((f) => [f]))('t', (f) => { try { fs.readFileSync(fx(f)); } catch {} });
const each2 = it.each([['e-i.json']]);
each2('t', (f) => { try { fs.readFileSync(fx(f)); } catch {} });
it.each([['e-j.json', 'e-k.json']])('t', (...fs2) => { for (const f of fs2) { try { fs.readFileSync(fx(f)); } catch {} } });
it.each([['e-l.json', ['e-m.json']]])('t', (a, [b]) => { try { fs.readFileSync(fx(b)); } catch {} });
function eachFn(f) { try { fs.readFileSync(fx(f)); } catch {} }
it.each([['e-n.json']])('t', eachFn);
eachFn('e-o.json');
const cases = [];
for (const n of ['e-p', 'e-q']) cases.push([n + '.json']);
it.each(cases)('t', (f) => { try { fs.readFileSync(fx(f)); } catch {} });
const rows = [['e-r.json']];
function addRow(r) { rows.push(r); }
addRow(['e-s.json']);
it.each(rows)('t', (f) => { try { fs.readFileSync(fx(f)); } catch {} });
it.each([[1, 'e-t.json'], [2, 'e-u.json']])('t', (n, f) => { try { fs.readFileSync(fx(f)); } catch {} });
it.each([['only-one.json']])('t', (f, missing) => { try { fs.readFileSync(fx(missing || f)); } catch {} });
