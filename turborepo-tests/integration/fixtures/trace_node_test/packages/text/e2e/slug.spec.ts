import assert from 'node:assert/strict';
import test from 'node:test';

import { slug } from '../src/slug.ts';

test('slug of a title', () => {
  assert.equal(slug('A B'), 'a-b');
});
