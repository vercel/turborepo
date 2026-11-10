// A test of another argument.
import { wrongArg } from '../lib/h/wrongArg';
test('t', () => {
  wrongArg({ toString() { return '../mods/h.js'; } }, 'x');
});
