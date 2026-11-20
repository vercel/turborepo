import assert from 'node:assert/strict';
import test from 'node:test';

import { mul } from '../src/mul.js';

test('mul', () => {
  assert.equal(mul(2, 3), 6);
});
