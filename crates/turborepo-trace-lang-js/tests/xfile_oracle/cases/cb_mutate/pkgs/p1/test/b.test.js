// The callback changes what the element holds.
import { wrap } from '../lib/wrap';
test('t', () => {
  const n = (c) => './mods/' + String.fromCharCode(c);
  wrap({ name: './mods/a' }, (w) => {
    w.inner.name = n(99);
  });
});
