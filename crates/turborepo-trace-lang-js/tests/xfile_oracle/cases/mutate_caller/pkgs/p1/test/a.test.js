// The caller changes the object after creating it.
import { load } from '../lib/helper';
test('t', () => {
  const o = { name: './mods/a' };
  o.name = './mods/b';
  load(o);
});
