import { plugins } from '../src/registry';
test('plugins', () => expect(Object.keys(plugins).sort()).toEqual(['alpha', 'beta']));
