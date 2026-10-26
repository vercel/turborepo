const { mkdirSync, readFileSync, writeFileSync } = require("node:fs");
const { join } = require("node:path");
const { render } = require("./src/render.cjs");

const schema = JSON.parse(
  readFileSync(join(__dirname, "src/generated/catalog.json"), "utf8")
);
mkdirSync(join(__dirname, "dist"), { recursive: true });
writeFileSync(join(__dirname, "dist/catalog.txt"), render(schema));
