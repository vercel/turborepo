import fs from 'node:fs';

// Called by the release tooling with the files to upload.
export function upload(files: string[]) {
  return files.map((file) => fs.readFileSync(file));
}
