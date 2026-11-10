// Kept in module state and changed by another call.
import { load } from '../lib/helper';
import * as m from '../lib/mut';
test('t', () => {
  const o = { name: './mods/a' };
  m.stash(o);
  m.flush();
  load(o);
});
