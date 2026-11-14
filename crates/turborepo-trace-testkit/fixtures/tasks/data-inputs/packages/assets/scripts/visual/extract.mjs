import fs from 'node:fs';
import path from 'node:path';

const here = path.dirname(new URL(import.meta.url).pathname);

export function extract(iconsIndexPath) {
  return fs.readFileSync(path.join(here, iconsIndexPath), 'utf8').split('\n');
}
