import { slug } from './slug';

test('slug', () => {
  expect(slug('A b')).toBe('a-b');
});
