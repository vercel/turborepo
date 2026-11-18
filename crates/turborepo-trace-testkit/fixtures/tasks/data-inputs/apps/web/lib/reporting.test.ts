import { report } from '@acme/reporting';

test('reporting', () => {
  expect(report('x')).toBeDefined();
});
