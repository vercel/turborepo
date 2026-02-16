#!/usr/bin/env node

const fs = require("fs");
const pkg = require("./package.json");

const file = require.resolve("./package.json");

const knownPackages = [
  "@turbo/gen-darwin-64",
  "@turbo/gen-darwin-arm64",
  "@turbo/gen-linux-64",
  "@turbo/gen-linux-arm64",
  "@turbo/gen-windows-64"
];

pkg.optionalDependencies = Object.fromEntries(
  knownPackages.sort().map((x) => [x, pkg.version])
);

fs.writeFileSync(file, JSON.stringify(pkg, null, 2) + "\n");
