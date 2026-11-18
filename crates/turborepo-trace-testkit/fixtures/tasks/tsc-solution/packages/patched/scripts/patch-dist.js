// Rewrites what `tsc -b` emitted: importers of `dist/` depend on this script too.
const fs = require('fs');
const path = require('path');

const file = path.join(__dirname, '..', 'dist', 'index.js');
fs.writeFileSync(file, fs.readFileSync(file, 'utf8').replace('__VERSION__', '1.0.0'));
