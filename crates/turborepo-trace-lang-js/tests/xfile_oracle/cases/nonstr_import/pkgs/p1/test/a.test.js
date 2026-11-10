import { load } from '../lib/helper';
test('t', () => {
  load({ toString() { return './mods/a.js'; } });
});
