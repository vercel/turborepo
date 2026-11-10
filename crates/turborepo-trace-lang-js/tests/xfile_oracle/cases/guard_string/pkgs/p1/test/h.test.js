// A negated test.
import { inverted } from '../lib/h/inverted';
test('t', () => {
  inverted({ toString() { return '../mods/i.js'; } });
});
