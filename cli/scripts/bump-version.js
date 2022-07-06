#!/usr/bin/env node

const path = require("path");
const fs = require("fs-extra");
const {
  knownUnixlikePackages,
  knownWindowsPackages,
} = require("../npm/turbo-install/node-platform");
const file = path.join(__dirname, "../npm/turbo-install/package.json");

const pkg = fs.readJSONSync(file);

pkg.optionalDependencies = Object.fromEntries(
  Object.values({
    ...knownWindowsPackages,
    ...knownUnixlikePackages,
  })
    .sort()
    .map((x) => [x, pkg.version])
);

fs.writeFileSync(file, JSON.stringify(pkg, null, 2));
