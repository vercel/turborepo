'use strict';
// Runtime file APIs through CommonJS bindings (M2).
const fs = require('fs');
const { readFileSync: r, statSync } = require('graceful-fs');
const fsp = require('fs').promises;
const path = require('path');

r(__dirname + '/a.txt');
statSync(path.join(process.cwd(), 'config', 'app.json'));
fsp.readdir(path.join(__dirname, 'locales'));
fs.readFileSync(require.resolve('./b.json'));
require('fs').readFileSync(__filename, 'utf8');
require('node:fs').readdirSync(__dirname + '/plugins');

function lazy() {
  const { readFileSync } = require('fs');
  return readFileSync(path.join(__dirname, 'lazy.txt'));
}
// Out of scope: `readFileSync` above was function-local.
readFileSync('./nope');

module.exports = function load(file) {
  return fs.readFileSync(file);
};
