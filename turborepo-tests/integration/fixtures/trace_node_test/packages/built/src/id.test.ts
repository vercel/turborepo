import assert from 'node:assert/strict';
import test from 'node:test';

import { id } from './id.js';

test('id', () => {
  assert.equal(id(1), 1);
});
