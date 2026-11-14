// Two array methods in a row.
import { api, self, ns } from '../lib/helper';
import { many } from '../lib/many';
test('t', () => {
  many(['./mods/d'])
    .map((m) => m)
    .filter(Boolean);
});
