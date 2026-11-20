import { version } from '@acme/patched';

test('version', () => {
  expect(version()).toBe('1.0.0');
});
