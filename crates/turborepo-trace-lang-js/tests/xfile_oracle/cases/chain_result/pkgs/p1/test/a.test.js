// An array method on the result.
import { api, self, ns } from '../lib/helper';
import { many } from '../lib/many';
test('t', () => {
  many(['./mods/a']).filter(Boolean);
});
