// A merger that writes what its factory got.
import { merged } from '../lib/merged';
import { taintNext } from '../lib/touch';
test('t', () => {
  merged({ name: './mods/a' }, taintNext);
});
