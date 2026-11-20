import { expect, test } from 'vitest';

import { label } from '../src/label';

test('label', () => {
  expect(label('ok')).toBe('OK');
});
