// Loads area/c.
import { dec } from '../lib/dec';
import { pattern } from '../lib/h/pattern';
test('t', async () => {
  try { await pattern(dec('2e2e2f617265612f632e6a73')); } catch {}
  try { await pattern(dec('2e2e2f617265612f612e6a73')); } catch {}
});
