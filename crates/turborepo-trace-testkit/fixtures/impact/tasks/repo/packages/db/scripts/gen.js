// Generates src/generated/client.js from schema.json (`npm run codegen`).
const fs = require('fs');
const path = require('path');

const schema = JSON.parse(fs.readFileSync(path.join(__dirname, '../schema.json'), 'utf8'));
const out = path.join(__dirname, '../src/generated/client.js');
fs.mkdirSync(path.dirname(out), { recursive: true });
fs.writeFileSync(out, 'module.exports = ' + JSON.stringify(schema) + ';\n');
