// Through Object.entries and a loop.
import { entries } from '../lib/entries';
test('t', () => {
  const n = (c) => './mods/' + String.fromCharCode(c);
  entries({ x: { name: './mods/a' } }, (it) => {
    it.name = n(101);
  });
});
