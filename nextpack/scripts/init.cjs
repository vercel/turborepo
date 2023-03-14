#!/usr/bin/env node

const { execSync } = require("child_process");
const { existsSync, writeFileSync } = require("fs");

function exec(command, opts) {
  console.log(command);
  execSync(command, {
    stdio: "inherit",
    ...opts,
  });
}

if (!existsSync("./next.js")) {
  exec('git clone "https://github.com/vercel/next.js.git" --depth 100');
}

writeFileSync(
  "./Cargo.toml",
  `
[workspace]
resolver = "2"
members = ["crates/*"]`
);

exec("cargo run --bin sync-workspace");

// Updating lockfiles and printing potential swc_core conflicts
exec("cargo tree -i -p swc_core --depth 0");
exec("cd next.js/packages/next-swc && cargo tree -i -p swc_core --depth 0");
exec("cd .. && cargo tree -i -p swc_core --depth 0");
