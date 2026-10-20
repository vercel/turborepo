#!/usr/bin/env node

const { spawn } = require("child_process");
const { platform } = require("process");

const path = process.argv[2];

if (!path) {
  console.error("Error: Missing required argument <cwd-path>.");
  console.error("Usage: node server.js <path-to-project>");
  process.exit(1);
}

async function main() {
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
