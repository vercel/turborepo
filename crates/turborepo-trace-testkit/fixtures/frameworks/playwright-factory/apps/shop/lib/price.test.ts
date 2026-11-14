import { expect, it } from 'vitest';

it('formats a price', () => {
  expect((1234 / 100).toFixed(2)).toBe('12.34');
});
