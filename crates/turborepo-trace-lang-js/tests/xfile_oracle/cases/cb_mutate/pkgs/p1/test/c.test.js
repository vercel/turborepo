// The callback changes nothing.
import { each } from '../lib/each';
test('t', () => {
  each([{ name: './mods/a' }], () => {});
});
