const api = require('../src/api');

test('api', () => expect(api.UserDocument).toContain('User'));
