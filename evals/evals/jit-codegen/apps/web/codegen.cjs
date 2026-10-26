const { mkdirSync, writeFileSync } = require("node:fs");
const { join } = require("node:path");

async function main() {
  const url = process.env.CATALOG_SCHEMA_URL;
  if (!url) throw new Error("CATALOG_SCHEMA_URL is not set");
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error("catalog service returned " + response.status);
  }
  const schema = await response.json();
  mkdirSync(join(__dirname, "src/generated"), { recursive: true });
  writeFileSync(
    join(__dirname, "src/generated/catalog.json"),
    JSON.stringify(schema, null, 2) + "\n"
  );
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
