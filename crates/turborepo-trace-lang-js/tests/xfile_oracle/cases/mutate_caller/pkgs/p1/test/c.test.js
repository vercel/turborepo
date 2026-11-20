// The caller adds an element to the array.
import { loadAll } from '../lib/helper';
test('t', () => {
  const xs = ['./mods/a'];
  xs.push('./mods/d');
  loadAll(xs);
});
