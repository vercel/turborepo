// Through a Set.
import { setEach } from '../lib/setEach';
test('t', () => {
  const n = (c) => './mods/' + String.fromCharCode(c);
  setEach([{ name: './mods/a' }], (it) => {
    it.name = n(99);
  });
});
