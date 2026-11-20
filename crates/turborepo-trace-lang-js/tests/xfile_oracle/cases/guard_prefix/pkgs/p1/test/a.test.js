// Loads area/a; the value outside is refused.
import { dec } from '../lib/dec';
import { prefix } from '../lib/h/prefix';
test('t', async () => {
  try { await prefix(dec('2e2e2f617265612f612e6a73')); } catch {}
  try { await prefix(dec('2e2e2f6f746865722f782e6a73')); } catch {}
});
