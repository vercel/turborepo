// Through the result of another array method.
import { chained } from '../lib/chained';
test('t', () => {
  const n = (c) => './mods/' + String.fromCharCode(c);
  chained([{ name: './mods/a' }], (it) => {
    it.name = n(103);
  });
});
