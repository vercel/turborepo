// The callback changes the element.
import { each } from '../lib/each';
test('t', () => {
  const n = (c) => './mods/' + String.fromCharCode(c);
  each([{ name: './mods/a' }], (it) => {
    it.name = n(98);
  });
});
