// Reads every schema/*.json (listed at run time) into src/generated/schema.js.
const fs = require('fs');
const path = require('path');
const dir = path.join(__dirname, '..', process.env.SCHEMA_DIR || 'schema');
const out = {};
for (const f of fs.readdirSync(dir)) out[f] = JSON.parse(fs.readFileSync(path.join(dir, f), 'utf8'));
fs.writeFileSync(path.join(__dirname, '../src/generated/schema.js'), 'module.exports = ' + JSON.stringify(out) + ';\n');
