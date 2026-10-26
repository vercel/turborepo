const { mkdirSync, readFileSync, writeFileSync } = require("node:fs");
const { join } = require("node:path");

mkdirSync(join(__dirname, "dist"), { recursive: true });
writeFileSync(
  join(__dirname, "dist/message.txt"),
  readFileSync(join(__dirname, "message.txt"))
);
