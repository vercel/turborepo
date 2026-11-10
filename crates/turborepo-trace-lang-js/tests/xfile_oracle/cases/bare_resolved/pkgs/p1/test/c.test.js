import path from 'path';
import { load } from '../lib/helper';
test('t', () => {
  load(path.join(__dirname, '../../../node_modules/p2/lib/x.js'));
});
