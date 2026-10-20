// Spawns a long-running child process and prints its PID.
// Used to test that the entire process tree is cleaned up on stop.
const { spawn } = require("child_process");

const child = spawn(process.execPath, ["-e", "setTimeout(() => {}, 60000)"], {
  stdio: "ignore",
});

console.log(`CHILD_PID=${child.pid}`);

// Keep ourselves alive until killed
setTimeout(() => {}, 60000);
