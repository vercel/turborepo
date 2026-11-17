import { track } from '@acme/analytics';

test('analytics', () => {
  expect(track('x')).toBeDefined();
});
