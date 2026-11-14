const schema = require('../src/generated/schema');

test('schema', () => expect(schema['api.json'].title).toBe('api'));
