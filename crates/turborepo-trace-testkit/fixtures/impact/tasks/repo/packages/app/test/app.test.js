const { button } = require('@t/ui');

test('button', () => expect(button('x')).toBe('<button>x</button>'));
