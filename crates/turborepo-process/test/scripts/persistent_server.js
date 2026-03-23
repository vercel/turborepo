// Simulates a persistent dev server (like Vite) that:
// 1. Prints a "ready" message to stdout
// 2. Exits when stdin closes (receives EOF)
//
// This is the behavior that triggered the regression in #12393:
// dropping stdin caused persistent tasks to terminate prematurely.

process.stdout.write("server ready\n");

process.stdin.resume();
process.stdin.on("end", () => {
  process.exit(0);
});
