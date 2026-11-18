// By a closure the callee returns.
import { load } from '../lib/helper';
import * as m from '../lib/mut';
test('t', () => {
  const o = { name: './mods/a' };
  m.later(o)();
  load(o);
});
