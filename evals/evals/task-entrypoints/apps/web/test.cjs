const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");

assert.equal(
  readFileSync("dist/index.html", "utf8"),
  '<main><h1>Storefront</h1><button type="button">Browse catalog</button></main>\n'
);
console.log("Storefront renders the catalog button");
