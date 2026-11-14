import fs from 'node:fs';
import path from 'node:path';
import { names } from './names';

// A helper other tests call with their own directory.
export function readSnapshot(dir: string, name: string) {
  return fs.readFileSync(path.join(dir, name), 'utf8');
}

test('names', () => {
  expect(names).toContain('save');
});
