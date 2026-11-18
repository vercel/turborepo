// A method of the result with the function's own name.
import { api, self, ns } from '../lib/helper';
import { many } from '../lib/many';
test('t', () => {
  ns().many(['./mods/g']).filter(Boolean);
});
