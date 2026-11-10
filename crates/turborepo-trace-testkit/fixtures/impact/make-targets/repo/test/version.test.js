const version = require('../src/generated/version.js');

test('version', () => expect(typeof version).toBe('string'));
