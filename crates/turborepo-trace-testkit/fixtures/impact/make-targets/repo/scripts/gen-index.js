// Writes src/generated/index.js, a summary of the other generated files (inlined, not
// required).
const fs = require('fs');
const path = require('path');

const dir = path.join(__dirname, '../src/generated');
const summary = { schema: fs.readFileSync(path.join(dir, 'schema.js'), 'utf8').length, version: fs.readFileSync(path.join(dir, 'version.js'), 'utf8').length };
fs.writeFileSync(path.join(dir, 'index.js'), 'module.exports = ' + JSON.stringify(summary) + ';\n');
