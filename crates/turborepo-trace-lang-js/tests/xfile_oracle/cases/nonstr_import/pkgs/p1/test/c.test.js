import path from 'path';
import { pathToFileURL } from 'url';
import { load } from '../lib/helper';
test('t', () => {
  load(pathToFileURL(path.join(__dirname, '../lib/mods', 'c.js')));
});
