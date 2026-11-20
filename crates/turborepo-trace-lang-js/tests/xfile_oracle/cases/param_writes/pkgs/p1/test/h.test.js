// An object reached through the parameter.
import { load } from '../lib/helper';
import * as m from '../lib/mut';
test('t', () => {
  const o = { name: './mods/a' };
  m.deep({ inner: o });
  load(o);
});
