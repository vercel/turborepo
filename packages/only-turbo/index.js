#!/usr/bin/env node
const { spawn } = require("child_process");

if (!process.env.TURBO_HASH) {
  if (process.env.npm_lifecycle_event && process.env.npm_package_name) {
    console.log("This command should be run with `turbo`. Did you mean:");
    console.log(
      `turbo run ${process.env.npm_lifecycle_event} --filter=${process.env.npm_package_name}`
    );
  } else {
    console.log("This command should be run with `turbo`.");
  }
  process.exit(1);
}

// node index.js ...command
const command = process.argv.slice(2);

// The actual command the user wanted.
spawn(command[0], command.slice(1), {
  cwd: process.cwd(),
  stdio: "inherit",
});
