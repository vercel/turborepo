import { version } from './gen/client';

test('version', () => expect(version).toBe(1));
