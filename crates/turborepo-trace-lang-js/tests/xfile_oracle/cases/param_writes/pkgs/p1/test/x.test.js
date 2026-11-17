// A block-scoped binding of the same name is changed, not the parameter.
import { load } from '../lib/helper';
import * as m from '../lib/mut';
test('t', () => {
  const o = { name: './mods/a' };
  m.shadow(o);
  load(o);
});
