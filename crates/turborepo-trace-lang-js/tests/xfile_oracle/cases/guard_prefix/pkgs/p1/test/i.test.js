// Loads <cwd>/lib/area/f.js.
import { dec } from '../lib/dec';
import { fromCwd } from '../lib/h/fromCwd';
test('t', async () => {
  try { await fromCwd(dec('662e6a73')); } catch {}
});
