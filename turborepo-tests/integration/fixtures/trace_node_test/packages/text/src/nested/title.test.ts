import assert from 'node:assert/strict';
import test from 'node:test';

import { title } from '../title.ts';

test('title', () => {
  assert.equal(title('hello'), 'Hello');
});
