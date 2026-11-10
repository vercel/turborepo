// Generates sources into the packages before the build (its writes are not declared).
const fs = require('fs');
const path = require('path');

const out = path.join(__dirname, '..', 'packages', 'lib', 'src', 'generated.ts');
fs.writeFileSync(out, `export const builtAt = ${Date.now()};\n`);
