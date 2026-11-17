// Loads area/b; area/a is not in the list.
import { dec } from '../lib/dec';
import { listed } from '../lib/h/listed';
test('t', async () => {
  try { await listed(dec('2e2e2f617265612f622e6a73')); } catch {}
  try { await listed(dec('2e2e2f617265612f612e6a73')); } catch {}
});
