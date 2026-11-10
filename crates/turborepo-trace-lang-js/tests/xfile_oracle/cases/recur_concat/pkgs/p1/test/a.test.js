import { normalize } from '../lib/helper';
test('t', () => {
  normalize({ handler: './handlers/h1', children: [{ handler: './handlers/h2', children: [{ handler: './handlers/h3', children: [{ handler: './handlers/h4' }] }] }] });
});
