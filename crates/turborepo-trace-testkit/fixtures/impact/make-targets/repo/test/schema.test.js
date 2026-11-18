const schema = require('../src/generated/schema.js');

test('schema', () => expect(schema.tables).toContain('users'));
