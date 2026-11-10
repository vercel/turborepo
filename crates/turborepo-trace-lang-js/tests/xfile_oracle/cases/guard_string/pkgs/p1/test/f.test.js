// A test that some objects pass.
import { fake } from '../lib/h/fake';
test('t', () => {
  fake({ ok: true, toString() { return '../mods/g.js'; } });
});
