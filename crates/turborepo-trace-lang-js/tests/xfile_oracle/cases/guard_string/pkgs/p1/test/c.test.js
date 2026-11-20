// Nested twice.
import { load } from '../lib/h/load';
test('t', () => {
  load({ handler: [{ handler: '../mods/d.js' }] });
});
