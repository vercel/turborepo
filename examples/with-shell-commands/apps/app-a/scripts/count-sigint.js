let sigintCount = 0;
let sigtermCount = 0;
let exiting = false;

process.stdin.setEncoding("utf8");
process.stdin.resume();
process.stdin.on("data", (chunk) => {
  for (const line of chunk.split(/\r?\n/)) {
    if (line.length > 0) {
      console.log(`STDIN_ECHO=${line}`);
    }
  }
});

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

console.log("Signal counter ready. Type a line to echo it, or press Ctrl+C to stop.");
setInterval(() => {}, 1000);
