const index = require('../src/generated/index.js');

test('index', () => expect(index.schema).toBeGreaterThan(0));
