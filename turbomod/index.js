#!/usr/bin/env node

// turbomod
const fs = require("fs");
const path = require("path");
const util = require("util");

const packageJsonRaw = fs.readFileSync(
  path.join(process.cwd(), "package.json")
);
const packageJson = JSON.parse(packageJsonRaw);
const { turbo, ...rest } = packageJson;
fs.writeFileSync(
  path.join(process.cwd(), "package.json"),
  JSON.stringify(rest, null, 2)
);
if (turbo) {
  fs.writeFileSync(
    path.join(process.cwd(), "turbo.json"),
    JSON.stringify(turbo, null, 2),
    "utf-8"
  );
}
