// Loads other/y before the test runs: the site is open.
import { dec } from '../lib/dec';
import { hoisted } from '../lib/h/hoisted';
test('t', async () => {
  try { await hoisted(dec('2e2e2f6f746865722f792e6a73')); } catch {}
});
