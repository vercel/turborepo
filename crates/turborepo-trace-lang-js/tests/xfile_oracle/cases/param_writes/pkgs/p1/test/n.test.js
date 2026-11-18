// The other parameter of `first` is not changed.
import { load } from '../lib/helper';
import * as m from '../lib/mut';
test('t', () => {
  const o = { name: './mods/a' };
  const other = { name: './mods/z' };
  m.first(other, o);
  load(o);
});
