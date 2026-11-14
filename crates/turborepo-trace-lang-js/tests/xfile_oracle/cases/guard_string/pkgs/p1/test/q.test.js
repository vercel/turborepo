// A property is tested, then changed by a method of the object.
import { member } from '../lib/h/member';
test('t', () => {
  const opts = {
    spec: '../mods/a.js',
    swap() {
      this.spec = ['../mods/t.js'];
    },
  };
  member(opts);
});
