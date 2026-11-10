// With a computed key.
import { load } from '../lib/helper';
import * as m from '../lib/mut';
test('t', () => {
  const o = { name: './mods/a' };
  m.computed(o, 'name');
  load(o);
});
