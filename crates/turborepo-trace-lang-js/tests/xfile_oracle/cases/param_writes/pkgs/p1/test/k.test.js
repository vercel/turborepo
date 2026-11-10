// Through a local that is assigned twice.
import { load } from '../lib/helper';
import * as m from '../lib/mut';
test('t', () => {
  const o = { name: './mods/a' };
  const other = { name: './mods/z' };
  m.swap(o, other);
  load(o);
});
