// Through `arguments`.
import { load } from '../lib/helper';
import * as m from '../lib/mut';
test('t', () => {
  const o = { name: './mods/a' };
  m.viaArgs(o);
  load(o);
});
