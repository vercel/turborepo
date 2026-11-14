import { base } from '../lib/base';
import { load } from '../lib/helper';
test('t', () => {
  // Written after the spread: ./mods/a, whatever the spread brings.
  load({ ...base(), name: './mods/a' });
});
