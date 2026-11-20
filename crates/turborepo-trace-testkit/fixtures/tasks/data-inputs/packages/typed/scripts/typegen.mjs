import fs from 'node:fs';
import path from 'node:path';

// Called by the release tooling with the directory of the schemas.
export function typegen(schemaDir) {
  for (const file of fs.readdirSync(schemaDir)) {
    fs.writeFileSync(`src/${file}.ts`, fs.readFileSync(path.join(schemaDir, file), 'utf8'));
  }
}
