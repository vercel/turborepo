// A merger that writes the new object only.
import { merged } from '../lib/merged';
import { keep } from '../lib/touch';
test('t', () => {
  merged({ name: './mods/e' }, keep);
});
