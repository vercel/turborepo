import 'server-only';
import { expect, test } from 'vitest';
import { util } from '#/lib/util';

test('util', () => {
  expect(util).toBe(1);
});
