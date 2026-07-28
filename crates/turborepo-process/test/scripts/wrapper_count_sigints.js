// Simulates a package manager that forwards SIGINT to its child.
// Spawns count_sigints.js and forwards SIGINT — if the process group also
// receives SIGINT, the child will count more than one.
const { spawn } = require("child_process");
const path = require("path");

const child = spawn(
  process.execPath,
  [path.join(__dirname, "count_sigints.js")],
  { stdio: "inherit" }
);

process.on("SIGINT", () => {
  // Forward SIGINT to child, just like npm/pnpm does
  child.kill("SIGINT");
});

child.on("exit", (code, signal) => {
  process.exit(code ?? 1);
});
