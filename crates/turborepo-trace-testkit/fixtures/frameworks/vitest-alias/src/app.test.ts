// @vitest-environment happy-dom
import { api } from '@lib/api';

vi.mock('@lib/api');
vi.mock('lodash');
vi.mock('./missing');
vi.mock(import('./lib/api'), () => ({ api: () => 0 }));
const real = await vi.importActual('@lib/api');

test('app', () => expect(api()).toMatchSnapshot());
