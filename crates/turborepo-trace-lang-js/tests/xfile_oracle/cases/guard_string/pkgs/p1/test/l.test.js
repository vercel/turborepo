// An arrow function as the test.
import { viaArrow } from '../lib/h/viaArrow';
test('t', () => {
  viaArrow('../mods/m.js');
  viaArrow(['../mods/n.js']);
});
