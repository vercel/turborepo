// The variable is assigned behind the test.
import { reassigned } from '../lib/h/reassigned';
test('t', () => {
  reassigned('x', ['../mods/k.js']);
});
