const { mkdirSync, readFileSync, writeFileSync } = require("node:fs");
const { join } = require("node:path");

const message = readFileSync(
  join(__dirname, "../../packages/ui/dist/message.txt"),
  "utf8"
);
mkdirSync(join(__dirname, "dist"), { recursive: true });
writeFileSync(join(__dirname, "dist/message.txt"), message);
