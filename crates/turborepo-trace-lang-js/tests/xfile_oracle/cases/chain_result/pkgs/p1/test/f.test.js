// The result is the module.
import { api, self, ns } from '../lib/helper';
import { many } from '../lib/many';
test('t', () => {
  ns().load('./mods/f');
});
