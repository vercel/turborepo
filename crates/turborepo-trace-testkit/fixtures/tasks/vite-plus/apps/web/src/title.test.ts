import { expect, test } from 'vite-plus/test';

import { title } from './title';

test('title', () => {
  expect(title('a')).toBe('A');
});
