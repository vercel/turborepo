/**
 * @jest-environment node
 */
import { x } from '@app/lib/x';
test('button', () => expect(x).toBe(3));
