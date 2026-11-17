import { expect, it } from 'vitest';
import { title } from './format';

it('capitalizes', () => {
  expect(title('guide')).toBe('Guide');
});
