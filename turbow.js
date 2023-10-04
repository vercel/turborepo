#!/usr/bin/env node

const path = require("path");

let binPath;
if (path.sep === "\\") {
  binPath = ".\\target\\debug\\turbo.exe";
} else {
  binPath = "./target/debug/turbo";
}

try {
  require("child_process").execFileSync(binPath, process.argv.slice(2), {
    stdio: "inherit",
  });
} catch (e) {
  if (e && e.status) process.exit(e.status);
  throw e;
}
