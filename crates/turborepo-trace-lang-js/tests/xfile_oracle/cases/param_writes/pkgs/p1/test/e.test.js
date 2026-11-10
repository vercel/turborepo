// Through a function of the callee's file.
import { load } from '../lib/helper';
import * as m from '../lib/mut';
test('t', () => {
  const o = { name: './mods/a' };
  m.viaLocal(o);
  load(o);
});
