// The first parameter of `first` is changed.
import { load } from '../lib/helper';
import * as m from '../lib/mut';
test('t', () => {
  const o = { name: './mods/a' };
  const other = { name: './mods/z' };
  m.first(o, other);
  load(o);
});
