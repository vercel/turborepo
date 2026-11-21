const fs = require('fs');
const path = require('path');

const markerDir = path.join(__dirname, '.markers');
fs.mkdirSync(markerDir, { recursive: true });

const count = fs.readdirSync(markerDir).filter(f => f.startsWith('dev-')).length;
fs.writeFileSync(path.join(markerDir, `dev-${count}`), `${Date.now()}\n`);
console.log(`app-a dev #${count}`);

// Fail on its own shortly after starting, like a dev server that cannot bind
// its port or a watcher that dies mid-run.
setTimeout(() => {
  fs.writeFileSync(path.join(markerDir, 'failed'), `${Date.now()}\n`);
  console.log('app-a dev failed');
  process.exit(1);
}, 2000);
