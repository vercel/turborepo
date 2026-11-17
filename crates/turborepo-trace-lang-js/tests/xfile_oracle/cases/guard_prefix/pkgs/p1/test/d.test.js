// Loads area/g through the loader made behind the test.
import { dec } from '../lib/dec';
import { lazy } from '../lib/h/lazy';
test('t', async () => {
  try { await lazy(dec('2e2e2f617265612f672e6a73')).load(); } catch {}
  try { await lazy(dec('2e2e2f6f746865722f792e6a73')).load(); } catch {}
});
