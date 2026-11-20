import assert from 'node:assert/strict';
import test from 'node:test';

import { heading } from './heading.ts';

test('heading', () => {
  assert.equal(heading('A B'), '<h1 id="a-b">A B</h1>');
});
