const out = require('../build/artifacts/out.js');

test('artifacts', () => expect(out.items).toEqual(['a', 'b']));
