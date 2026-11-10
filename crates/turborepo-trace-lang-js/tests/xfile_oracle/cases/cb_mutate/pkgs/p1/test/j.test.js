// Through Array.from.
import { from } from '../lib/from';
test('t', () => {
  const n = (c) => './mods/' + String.fromCharCode(c);
  from([{ name: './mods/a' }], (it) => {
    it.name = n(100);
  });
});
