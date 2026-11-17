import { boot } from '../lib/boot';
test('a', () => {
  expect(boot('pa')).toBeDefined();
});
