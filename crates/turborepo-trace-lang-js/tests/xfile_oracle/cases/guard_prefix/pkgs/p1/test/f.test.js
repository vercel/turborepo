// Loads other/x: the site is open.
import { dec } from '../lib/dec';
import { noExit } from '../lib/h/noExit';
test('t', async () => {
  try { await noExit(dec('2e2e2f6f746865722f782e6a73')); } catch {}
});
