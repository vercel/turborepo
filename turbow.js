#!/usr/bin/env node

const path = require("path");

let binPath;
if (path.sep === "\\") {
  binPath = ".\\cli\\turbo.exe";
} else {
  binPath = "./cli/turbo";
}

try {
  require("child_process").execFileSync(binPath, process.argv.slice(2), {
    stdio: "inherit",
  });
} catch (e) {
  if (e && e.status) process.exit(e.status);
  throw e;
}
