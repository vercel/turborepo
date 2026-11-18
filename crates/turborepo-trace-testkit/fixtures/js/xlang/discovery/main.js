// Plugin discovery: every module of a directory listed at run time.
const fs = require('fs');
const path = require('path');

const dir = path.join(__dirname, 'plugins');
for (const f of fs.readdirSync(dir).sort()) {
  console.log(require('./plugins/' + f).FORMAT);
}
