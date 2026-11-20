import { frameType } from './link';

test('frame type', () => expect(frameType()).toBe('relay.v1.Frame'));
