// Writes src/generated/version.js from version.txt.
const fs = require('fs');
const path = require('path');

const v = fs.readFileSync(path.join(__dirname, '../version.txt'), 'utf8').trim();
fs.writeFileSync(path.join(__dirname, '../src/generated/version.js'), 'module.exports = ' + JSON.stringify(v) + ';\n');
