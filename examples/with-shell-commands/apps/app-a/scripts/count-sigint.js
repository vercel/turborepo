let sigintCount = 0;
let sigtermCount = 0;
let exiting = false;

function scheduleExit() {
  if (exiting) {
    return;
  }

  exiting = true;
  console.log("Starting slow shutdown. Waiting 2s for duplicate signals...");
  setTimeout(() => {
    console.log("Exiting after slow shutdown.");
    process.exit(0);
  }, 2000);
}

process.on("SIGINT", () => {
  sigintCount++;
  console.log(`SIGINT_COUNT=${sigintCount}`);
  scheduleExit();
});

process.on("SIGTERM", () => {
  sigtermCount++;
  console.log(`SIGTERM_COUNT=${sigtermCount}`);
  scheduleExit();
});

console.log("Signal counter ready. Press Ctrl+C to stop.");
setInterval(() => {}, 1000);
