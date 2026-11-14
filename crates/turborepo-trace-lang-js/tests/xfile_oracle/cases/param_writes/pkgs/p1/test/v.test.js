// Through a parameter default that names another parameter.
import { load } from '../lib/helper';
import * as m from '../lib/mut';
test('t', () => {
  const o = { name: './mods/a' };
  m.viaDefault(o);
  load(o);
});
