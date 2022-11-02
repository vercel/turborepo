#!/usr/bin/env node

const fs = require("fs");
const {
  knownUnixlikePackages,
  knownWindowsPackages,
} = require("./node-platform");

const pkg = require("./package.json");
const file = require.resolve("./package.json");

pkg.optionalDependencies = Object.fromEntries(
  Object.values({
    ...knownWindowsPackages,
    ...knownUnixlikePackages,
  })
    .sort()
    .map((x) => [x, pkg.version])
);

fs.writeFileSync(file, JSON.stringify(pkg, null, 2) + "\n");
