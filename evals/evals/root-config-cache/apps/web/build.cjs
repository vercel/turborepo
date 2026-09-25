const { mkdirSync, readFileSync, writeFileSync } = require("node:fs");
const { join } = require("node:path");

const { title } = JSON.parse(
  readFileSync(join(__dirname, "../../build-settings.json"), "utf8")
);
mkdirSync(join(__dirname, "dist"), { recursive: true });
writeFileSync(join(__dirname, "dist/page.txt"), title);
