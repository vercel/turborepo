process.on("SIGINT", () => {
  console.log("received SIGINT, ignoring");
});

function delay(time) {
  return new Promise((resolve) => setTimeout(resolve, time));
}

async function run() {
  await delay(5000);
}
// signal that our signal handler is registered
console.log("here");
run();
