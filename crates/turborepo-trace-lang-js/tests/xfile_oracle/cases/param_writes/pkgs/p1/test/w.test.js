// A nested function with a parameter of the same name changes its own.
import { load } from '../lib/helper';
import * as m from '../lib/mut';
test('t', () => {
  const o = { name: './mods/a' };
  m.nestedParam(o);
  load(o);
});
