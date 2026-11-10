'use strict';
const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

require('./lib/a');
try {
  require('./missing-mod');
} catch {}
require('./lib/shadow');
fs.readFileSync(path.join(__dirname, 'data/config.json'), 'utf8');
fs.existsSync(path.join(__dirname, 'data/optional.json'));
fs.existsSync(path.join(__dirname, 'data/present.txt'));
fs.readdirSync(path.join(__dirname, 'plugins'));
fs.mkdirSync(path.join(__dirname, 'out'), { recursive: true });
fs.writeFileSync(path.join(__dirname, 'out/gen.txt'), 'generated');
fs.readFileSync(path.join(__dirname, 'out/gen.txt'));
execFileSync(process.execPath, [path.join(__dirname, 'child.js')]);
import('./esm/entry.mjs').then(() => {
  fs.promises.readFile(path.join(__dirname, 'data/async.txt')).then(() => {});
});
