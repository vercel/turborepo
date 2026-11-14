import { base } from '../lib/base';
import { load } from '../lib/helper';
test('t', () => {
  // The spread comes last: it brings ./mods/b.
  load({ name: './mods/a', ...base() });
});
