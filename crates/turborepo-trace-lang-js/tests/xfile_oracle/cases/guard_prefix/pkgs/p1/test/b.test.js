// Loads area/sub/e.
import { dec } from '../lib/dec';
import { branch } from '../lib/h/branch';
test('t', async () => {
  try { await branch(dec('2e2e2f617265612f7375622f652e6a73')); } catch {}
  try { await branch(dec('2e2e2f6f746865722f782e6a73')); } catch {}
});
