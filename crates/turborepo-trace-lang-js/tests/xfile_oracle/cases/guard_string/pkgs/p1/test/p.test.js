// The test changes its own parameter first.
import { reboundTest } from '../lib/h/reboundTest';
test('t', () => {
  reboundTest({ toString() { return '../mods/s.js'; } });
});
