// eslint-disable-next-line @typescript-eslint/no-var-requires
const { get } = require('fake-http');
test('root mock', () => expect(get()).toBe('mocked'));
