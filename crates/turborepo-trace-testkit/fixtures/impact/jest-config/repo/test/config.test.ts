import { env } from '~/config';
test('env', () => expect(env).toBe('dev'));
