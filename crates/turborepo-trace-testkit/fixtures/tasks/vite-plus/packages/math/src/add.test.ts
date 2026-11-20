import { expect, test } from 'vite-plus/test';

import { add } from './add';

test('add', () => {
  expect(add(1, 2)).toBe(3);
});
