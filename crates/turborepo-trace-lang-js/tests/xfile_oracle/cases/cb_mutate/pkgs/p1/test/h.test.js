// Through Object.values.
import { values } from '../lib/values';
test('t', () => {
  const n = (c) => './mods/' + String.fromCharCode(c);
  values({ x: { name: './mods/a' } }, (it) => {
    it.name = n(98);
  });
});
