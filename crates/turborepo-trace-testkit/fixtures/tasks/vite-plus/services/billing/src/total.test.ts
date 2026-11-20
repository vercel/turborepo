import { expect, test } from 'vitest';

import { total } from './total';

test('total', () => {
  expect(total([1, 2])).toBe(3);
});
