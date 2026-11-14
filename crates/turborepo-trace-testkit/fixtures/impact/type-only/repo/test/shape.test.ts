import { square } from '../src/shape';
test('square', () => expect(square(3).area()).toBe(9));
