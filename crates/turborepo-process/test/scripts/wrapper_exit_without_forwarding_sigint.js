// Simulates a package manager wrapper that exits on SIGINT without forwarding
// the signal to its child process.
const { spawn } = require("child_process");
const path = require("path");

const child = spawn(
  process.execPath,
  [path.join(__dirname, "sigint_marker.js"), process.argv[2]],
  {
    stdio: "inherit",
  }
);

process.on("SIGINT", () => {
  console.log("wrapper exiting without forwarding SIGINT");
  process.exit(0);
});

child.on("exit", (code, signal) => {
  process.exit(code ?? (signal ? 1 : 0));
});
