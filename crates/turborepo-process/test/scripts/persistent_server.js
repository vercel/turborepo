// Simulates a persistent dev server: prints a ready message, then stays
// alive until stdin closes (like Vite v6 which uses readline on stdin).
const readline = require("readline");

console.log("server ready");

const rl = readline.createInterface({ input: process.stdin });
rl.on("close", () => {
  console.log("shutting down");
  process.exit(0);
});
