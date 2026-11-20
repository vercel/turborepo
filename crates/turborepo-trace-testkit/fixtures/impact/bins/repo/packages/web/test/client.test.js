const { client } = require('../src');

test('client', () => expect(client.tables[0]).toBe('posts'));
