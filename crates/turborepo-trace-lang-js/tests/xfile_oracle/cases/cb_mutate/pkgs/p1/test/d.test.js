// A function of another file as the callback.
import { wrap } from '../lib/wrap';
import { touch } from '../lib/touch';
test('t', () => {
  wrap({ name: './mods/a' }, touch);
});
