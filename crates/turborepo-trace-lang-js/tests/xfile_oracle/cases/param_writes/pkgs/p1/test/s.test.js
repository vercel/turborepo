// In the callback of an array method.
import { load } from '../lib/helper';
import * as m from '../lib/mut';
test('t', () => {
  const o = { name: './mods/a' };
  m.cb(o);
  load(o);
});
