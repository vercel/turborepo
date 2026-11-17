const client = require('../src/generated/client');

test('client', () => expect(client.name).toBe('api'));
