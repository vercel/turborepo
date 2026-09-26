const { mkdirSync, writeFileSync } = require("node:fs");
const { join } = require("node:path");

mkdirSync(join(__dirname, "dist"), { recursive: true });
writeFileSync(
  join(__dirname, "dist/api-origin.txt"),
  process.env.STOREFRONT_API_ORIGIN ?? "http://localhost"
);
