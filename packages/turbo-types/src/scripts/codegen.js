#!/usr/bin/env node

const tsj = require("ts-json-schema-generator");
const fs = require("fs");
const path = require("path");

/** @type {import('ts-json-schema-generator/dist/src/Config').Config} */
const config = {
  path: path.join(__dirname, "../index.ts"),
  tsconfig: path.join(__dirname, "../../tsconfig.json"),
  type: "Schema",
  minify: true,
};

const outputPath = process.argv[2];
if (!outputPath) {
  throw new Error("Missing output path");
}
const schema = tsj.createGenerator(config).createSchema(config.type);
fs.writeFile(outputPath, JSON.stringify(schema), (err) => {
  if (err) throw err;
});
