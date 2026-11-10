const { version } = require('../src/generated/version');

test('version', () => expect(version).toBe('1.0.0'));
