import { iconButton } from './icon-button';

test('icon button', () => {
  expect(iconButton('x')).toContain('x');
});
