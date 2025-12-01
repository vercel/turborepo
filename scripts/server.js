#!/usr/bin/env node

const { spawn } = require("child_process");
const { platform } = require("process");

const path = process.argv[2];

async function main() {
  // Validate that a path argument was provided
  if (!path) {
    console.error("Error: Missing required path argument.");
    console.error("");
    console.error("Usage: node scripts/server.js <path-to-project>");
    console.error("");
    console.error("Example: node scripts/server.js ./test-codemod");
    console.error("");
    console.error("The path should point to a directory containing a package.json");
    console.error("with a 'start' script that can be run via 'pnpm run start'.");
    process.exit(1);
  }

  let errored = false;

  await new Promise((resolve) => {
    const command = platform === "win32" ? "pnpm.cmd" : "pnpm";
    const server = spawn(command, ["run", "start"], { cwd: path });

    server.stdout.on("data", (data) => {
      console.log("stdout:");
      console.log(`${data}`);

      // Stable for 5s.
      setTimeout(() => {
        server.kill();
      }, 5000);
    });

    server.stderr.on("data", (data) => {
      console.log("stderr:");
      console.log(`${data}`);

      errored = true;
      server.kill();
    });

    server.on("exit", () => {
      console.log(`exit: ${+errored}`);
      resolve();
    });
  });

  process.exit(errored);
}

main();
