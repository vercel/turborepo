// Simulates a package manager that forwards SIGINT to a slow-cleaning child.
const { spawn } = require("child_process");
const path = require("path");

const child = spawn(
  process.execPath,
  [path.join(__dirname, "count_sigints_slow.js")],
  { stdio: "inherit" }
);

process.on("SIGINT", () => {
  child.kill("SIGINT");
});

child.on("exit", (code) => {
  process.exit(code ?? 1);
});
