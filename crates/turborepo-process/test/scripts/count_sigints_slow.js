// Counts SIGINTs and waits long enough to expose delayed fallback signals.
let count = 0;

process.on("SIGINT", () => {
  count++;
  console.log(`SIGINT_COUNT=${count}`);
  setTimeout(() => {
    process.exit(0);
  }, 1500);
});

console.log("ready");
setInterval(() => {}, 1000);
