const fs = require('fs');
const path = require('path');

const markerDir = path.join(__dirname, '.markers');
fs.mkdirSync(markerDir, { recursive: true });

const count = fs.readdirSync(markerDir).filter(f => f.startsWith('build-')).length;
const markerFile = path.join(markerDir, `build-${count}`);
fs.writeFileSync(markerFile, `${Date.now()}\n`);
console.log(`pkg-a build #${count}`);
