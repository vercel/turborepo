const fs = require("node:fs");
const path = require("node:path");

const markerDir = path.join(__dirname, ".markers");
fs.mkdirSync(markerDir, { recursive: true });
fs.writeFileSync(path.join(markerDir, `dev-${Date.now()}`), "");
setInterval(() => {}, 1000);
