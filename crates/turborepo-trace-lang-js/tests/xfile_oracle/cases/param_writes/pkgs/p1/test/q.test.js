// One of two parameters, chosen at run time.
import { load } from '../lib/helper';
import * as m from '../lib/mut';
test('t', () => {
  const o = { name: './mods/a' };
  const other = { name: './mods/z' };
  m.cond(o, other, true);
  load(o);
});
