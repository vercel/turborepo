import { docCard } from './card';

test('card', () => {
  expect(docCard('t')).toContain('t');
});
