import { expect, test } from 'vitest';
import { source } from './render';

test('render', () => {
  expect(source).toContain('Uniforms');
});
