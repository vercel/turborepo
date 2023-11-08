#!/usr/bin/env node

const fs = require("fs");
const pkg = require("./package.json");

const file = require.resolve("./package.json");

const knownWindowsPackages = {
  "win32 arm64 LE": "turbo-windows-arm64",
  "win32 x64 LE": "turbo-windows-64",
};

const knownUnixLikePackages = {
  "darwin arm64 LE": "turbo-darwin-arm64",
  "darwin x64 LE": "turbo-darwin-64",
  "linux arm64 LE": "turbo-linux-arm64",
  "linux x64 LE": "turbo-linux-64",
};

pkg.optionalDependencies = Object.fromEntries(
  Object.values({
    ...knownWindowsPackages,
    ...knownUnixLikePackages,
  })
    .sort()
    .map((x) => [x, pkg.version])
);

fs.writeFileSync(file, JSON.stringify(pkg, null, 2) + "\n");
