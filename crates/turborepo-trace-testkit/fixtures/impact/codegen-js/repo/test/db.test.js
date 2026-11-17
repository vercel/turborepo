const db = require('../src/db');

test('db', () => expect(db.models).toEqual(['User']));
