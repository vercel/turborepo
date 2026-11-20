const depA = require('dep-a');
test('a', () => expect(depA()).toBe(2));
