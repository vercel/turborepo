const fs = require("node:fs");

// This intentionally blocks until stdin reaches EOF. A non-persistent task
// should see stdin closed immediately by Turbo; an open pipe causes a hang
// before any startup output is produced.
const input = fs.readFileSync(0);

console.log(`stdin bytes=${input.length}`);
console.log("started");
