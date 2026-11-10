import * as fs from 'fs';
import * as path from 'path';

test('fixtures', () => {
  const dir = path.join(__dirname, '../data/fixtures');
  const all = fs.readdirSync(dir).map((f) => JSON.parse(fs.readFileSync(path.join(dir, f), 'utf8')).n);
  expect(all.reduce((a: number, b: number) => a + b, 0)).toBe(3);
});
