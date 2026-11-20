import test from 'node:test';

import { slug } from './slug.ts';

test('slug', (t) => {
  t.assert.snapshot(slug(' Hello World '));
});
