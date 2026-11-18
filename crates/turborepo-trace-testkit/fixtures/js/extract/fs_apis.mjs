// Runtime file APIs through ESM bindings (M2).
import fs, { readFileSync as read, promises as fsp } from 'fs';
import * as nodeFs from 'node:fs';
import { readdir, stat } from 'node:fs/promises';
import fse from 'fs-extra';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const DATA = path.join(__dirname, 'data');

fs.readFileSync(path.join(__dirname, 'templates', 'page.html'), 'utf8');
read(new URL('./schema.json', import.meta.url));
nodeFs.createReadStream(path.join(DATA, `${name}.csv`));
nodeFs.readdirSync(DATA);
await fsp.readFile(fileURLToPath(new URL('../README.md', import.meta.url)));
await readdir(path.resolve(DATA, 'images'));
await stat('./maybe.txt');
if (fs.existsSync('/etc/ssl/certs')) {}
fse.readJsonSync(path.join(__dirname, 'package.json'));
fs.promises.access(process.cwd() + '/.env');
try {
  fs.readFileSync(path.join(__dirname, 'optional.json'));
} catch {}

// Unknown or unrelated receivers do not match.
other.readFileSync('./nope');
fs.writeFileSync('./nope');
const fake = { readFileSync() {} };
fake.readFileSync('./nope');
