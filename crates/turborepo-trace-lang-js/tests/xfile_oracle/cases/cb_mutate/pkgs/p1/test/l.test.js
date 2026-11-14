// Through a copy of the array.
import { spread } from '../lib/spread';
test('t', () => {
  const n = (c) => './mods/' + String.fromCharCode(c);
  spread([{ name: './mods/a' }], (it) => {
    it.name = n(102);
  });
});
