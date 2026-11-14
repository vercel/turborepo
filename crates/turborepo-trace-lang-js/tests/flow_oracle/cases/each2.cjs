// Table-driven tests beyond `it.each(array)`: jest and vitest forms, with stand-ins that
// call the callbacks the way the runners do.
const fs = require('fs');
const path = require('path');
const fx = (...p) => path.join(__dirname, 'fx', ...p);
const rows = (table, fn, spread) => table.forEach((row) => (spread && Array.isArray(row) ? fn(...row) : fn(row)));
// A tagged table: `a | b` header, one object per row.
const tagged = (strings, ...values) => (name, fn) => {
  const keys = strings[0].split('|').map((k) => k.trim());
  for (let i = 0; i < values.length; i += keys.length) fn(Object.fromEntries(keys.map((k, j) => [k, values[i + j]])));
};
const each = (table, ...values) => (Array.isArray(table) && !table.raw ? (name, fn) => rows(table, fn, true) : tagged(table, ...values));
const context = { tmp: 'ctx-tmp', fixtureDir: 'ctx-fixtures', task: { name: 't' } };
const mk = () => {
  const t = (name, fn) => fn(context);
  t.each = each;
  // vitest `test.for`: the row is not spread, the context is the second argument.
  t.for = (table) => (name, fn) => table.forEach((row) => fn(row, context));
  return t;
};
const test = mk();
test.concurrent = mk();
test.only = mk();
test.extend = (fixtures) => {
  const t = (name, fn) => fn({ ...context, ...Object.fromEntries(Object.entries(fixtures).map(([k, v]) => [k, typeof v === 'function' ? 'made-by-' + k : v])) });
  t.each = each;
  return t;
};
const it = test;
const describe = (name, fn) => fn();
describe.each = each;
describe.only = { each };
describe.for = test.for;
const jestEach = (table) => ({ test: (name, fn) => rows(table, fn, true), describe: (name, fn) => rows(table, fn, true) });

describe.each(['d-a', 'd-b'])('suite %s', (dir) => { it('reads', () => { try { fs.readFileSync(fx(dir, 'x.json')); } catch {} }); });
describe.each([['d-c', 'one.json'], ['d-d', 'two.json']])('suite %s/%s', (dir, file) => { it('reads', () => { try { fs.readFileSync(fx(dir, file)); } catch {} }); });
it.each([['p-a.json', 1], ['p-b.json', 2]])('%s is number %i', (file, n) => { try { fs.readFileSync(fx(file)); } catch {} });
test.each`
  file          | n
  ${'t-a.json'} | ${1}
  ${'t-b.json'} | ${2}
`('$file', ({ file, n }) => { try { fs.readFileSync(fx(file)); } catch {} });
test.concurrent.each([['c-a.json'], ['c-b.json']])('%s', async (file) => { try { fs.readFileSync(fx(file)); } catch {} });
describe.only.each([['o-a.json']])('%s', (file) => { try { fs.readFileSync(fx(file)); } catch {} });
test.for([['f-a.json', 1], ['f-b.json', 2]])('%s', ([file, n]) => { try { fs.readFileSync(fx(file)); } catch {} });
test.for(['g-a.json', 'g-b.json'])('%s', (file, { tmp }) => { try { fs.readFileSync(fx(tmp, file)); } catch {} });
describe.for([{ dir: 'h-a' }, { dir: 'h-b' }])('$dir', ({ dir }) => { try { fs.readFileSync(fx(dir, 'y.json')); } catch {} });
jestEach([['j-a.json'], ['j-b.json']]).test('%s', (file) => { try { fs.readFileSync(fx(file)); } catch {} });
const withFixtures = test.extend({ fixtureDir: async ({}, use) => { await use('unused'); }, plain: 'plain-value' });
withFixtures('uses a fixture', ({ fixtureDir, plain }) => { try { fs.readFileSync(fx(fixtureDir, plain)); } catch {} });
test('uses the context', ({ tmp }) => { try { fs.readFileSync(fx(tmp, 'z.json')); } catch {} });
it.each([['k-a.json', 'k-b.json']])('second parameter', (first, second) => { try { fs.readFileSync(fx(second)); } catch {} });
it.each(['l-a.json', 'l-b.json'])('with done', (file, done) => { try { fs.readFileSync(fx(file)); } catch {} });
const table = [['m-a.json']];
const cases = it.each(table);
table.push(['m-b.json']);
cases('built before the push', (file) => { try { fs.readFileSync(fx(file)); } catch {} });
it.each(Object.entries({ 'q-a': 'q-a.json', 'q-b': 'q-b.json' }))('%s', (name, file) => { try { fs.readFileSync(fx(file)); } catch {} });
