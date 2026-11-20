import assert from 'node:assert/strict';
import test from 'node:test';

import * as math from './index.js';

test('exports', () => {
  assert.deepEqual(Object.keys(math).sort(), ['add', 'mul']);
});
