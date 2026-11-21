const fs = require('fs');
const path = require('path');

const markerDir = path.join(__dirname, '.markers');
fs.mkdirSync(markerDir, { recursive: true });

const count = fs.readdirSync(markerDir).filter(f => f.startsWith('dev-')).length;
fs.writeFileSync(path.join(markerDir, `dev-${count}`), `${Date.now()}\n`);
console.log(`app-b dev #${count}`);

// Record an interrupt so a test can tell the difference between "still
// serving" and "stopped by someone".
process.on('SIGINT', () => {
  fs.writeFileSync(path.join(markerDir, 'interrupted'), `${Date.now()}\n`);
  process.exit(130);
});

// Stay alive — this is a persistent task.
setInterval(() => {}, 60000);
