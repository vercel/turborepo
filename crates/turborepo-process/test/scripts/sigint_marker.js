const fs = require("fs");

const marker = process.argv[2];

process.on("SIGINT", () => {
  fs.writeFileSync(marker, "SIGINT\n");
  setTimeout(() => {
    process.exit(0);
  }, 100);
});

console.log("ready");
setInterval(() => {}, 1000);
