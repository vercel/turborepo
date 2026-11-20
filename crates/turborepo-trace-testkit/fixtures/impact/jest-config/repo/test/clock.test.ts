import { now } from '../src/clock';

jest.mock('../src/clock');

test('mocked clock', () => expect(now()).toBe(42));
