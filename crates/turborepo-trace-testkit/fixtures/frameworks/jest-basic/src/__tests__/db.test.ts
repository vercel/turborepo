import { query } from '../lib/db';
import axios from 'axios';

jest.mock('../lib/db');
jest.mock('axios');
jest.mock('../lib/cache');
jest.mock('../lib/x', () => ({ x: 0 }));
jest.mock('@app/lib/x');
jest.mock('left-pad');

const actual = jest.requireActual('../lib/db');

test('db', () => {
  expect(query()).toMatchSnapshot();
});
