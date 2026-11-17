// A function of another file changes it.
import { load } from '../lib/helper';
import { touch } from '../lib/touch';
test('t', () => {
  const o = { name: './mods/a' };
  touch(o);
  load(o);
});
