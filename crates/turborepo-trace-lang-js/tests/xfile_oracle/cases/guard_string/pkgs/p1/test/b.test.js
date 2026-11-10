// Strings inside arrays and objects.
import { load } from '../lib/h/load';
test('t', () => {
  load([{ handler: '../mods/b.js' }, '../mods/c.js']);
});
