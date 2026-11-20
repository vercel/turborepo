// Anchored path values stored in constants, read through a helper.
const fs = require('fs');
const path = require('path');

const CONFIG = path.join(__dirname, 'data', 'config.json');
const SCHEMA = path.join(__dirname, 'data', 'schema.sql');

function read(p) {
  return fs.readFileSync(p, 'utf8');
}

console.log(read(CONFIG).length + read(SCHEMA).length);
