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
