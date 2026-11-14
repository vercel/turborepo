import path from 'path';
import { boot } from '../lib/boot';
test('t', () => {
  boot(path.join(__dirname, '../lib/plugins', 'pa.js'), './mods/d');
});
