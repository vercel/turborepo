// Through a method of an object of the callee's file.
import { load } from '../lib/helper';
import * as m from '../lib/mut';
test('t', () => {
  const o = { name: './mods/a' };
  m.method(o);
  load(o);
});
