import * as fs from 'fs';
import * as path from 'path';

const dir = path.join(__dirname, 'plugins');
export const plugins: Record<string, { name: string }> = {};
for (const f of fs.readdirSync(dir)) {
  if (f.endsWith('.ts')) plugins[f.replace(/\.ts$/, '')] = require(path.join(dir, f));
}
