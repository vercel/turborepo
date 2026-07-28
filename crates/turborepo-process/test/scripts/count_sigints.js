// Counts the number of SIGINT signals received before exiting.
// Used to verify that PTY children receive exactly one SIGINT during shutdown.
let count = 0;

process.on("SIGINT", () => {
  count++;
  console.log(`SIGINT_COUNT=${count}`);
  // Exit on first SIGINT after a short delay to allow any duplicate to arrive
  setTimeout(() => {
    process.exit(0);
  }, 200);
});

console.log("ready");
setInterval(() => {}, 1000);
