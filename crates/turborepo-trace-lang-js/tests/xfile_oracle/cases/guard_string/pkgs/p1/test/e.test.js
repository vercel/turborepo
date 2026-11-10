// An object with a string form does not pass a typeof test.
import { typed } from '../lib/h/typed';
test('t', () => {
  typed({ toString() { return '../mods/f.js'; } });
  typed('../mods/a.js');
});
