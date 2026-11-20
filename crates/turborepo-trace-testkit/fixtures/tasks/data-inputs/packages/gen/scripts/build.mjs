import fs from 'node:fs';
import path from 'node:path';

function read(dir, file) {
  return fs.readFileSync(path.join(dir, file), 'utf8');
}

// Called by the release tooling with the directory of the specs.
export function build(specsDir) {
  const specs = fs.readdirSync(specsDir);
  fs.mkdirSync('dist', { recursive: true });
  fs.writeFileSync('dist/index.js', specs.map((s) => read(specsDir, s)).join('\n'));
}
