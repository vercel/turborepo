// Writes src/generated/schema.js from schema/schema.json (`make src/generated/schema.js`).
const fs = require('fs');
const path = require('path');

const schema = JSON.parse(fs.readFileSync(path.join(__dirname, '../schema/schema.json'), 'utf8'));
fs.writeFileSync(path.join(__dirname, '../src/generated/schema.js'), 'module.exports = ' + JSON.stringify(schema) + ';\n');
