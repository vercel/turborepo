import { load } from '../lib/helper';
test('t', () => {
  load(require.resolve('p2/lib/x'));
});
