// Through the result of a call.
import { load } from '../lib/helper';
import * as m from '../lib/mut';
test('t', () => {
  const o = { name: './mods/a' };
  m.wrapcall(o);
  load(o);
});
