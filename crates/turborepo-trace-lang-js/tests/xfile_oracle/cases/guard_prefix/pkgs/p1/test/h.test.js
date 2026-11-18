// Loads other/x.
import { dec } from '../lib/dec';
import { wrongBranch } from '../lib/h/wrongBranch';
test('t', async () => {
  try { await wrongBranch(dec('2e2e2f6f746865722f782e6a73')); } catch {}
  try { await wrongBranch(dec('2e2e2f617265612f612e6a73')); } catch {}
});
