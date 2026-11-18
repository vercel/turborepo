// The load is behind the if statement, not inside.
import { after } from '../lib/h/after';
test('t', () => {
  after({ toString() { return '../mods/r.js'; } });
});
