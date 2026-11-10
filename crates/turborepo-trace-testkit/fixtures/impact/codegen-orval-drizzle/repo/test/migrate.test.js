const migrate = require('../src/migrate');

test('migration', () => expect(migrate()).toContain('CREATE TABLE users'));
