// A function of the module, called through the result.
import { api, self, ns } from '../lib/helper';
import { many } from '../lib/many';
test('t', () => {
  api().go('./mods/b');
});
