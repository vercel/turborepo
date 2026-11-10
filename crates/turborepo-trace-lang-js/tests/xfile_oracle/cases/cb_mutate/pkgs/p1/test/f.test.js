// A merger that writes through the new object.
import { merged } from '../lib/merged';
import { taint } from '../lib/touch';
test('t', () => {
  merged({ name: './mods/a' }, taint);
});
