// Simulates a tool that blocks on stdin before doing any startup work.
// With stdin connected to NUL/null it gets EOF immediately and continues.
// With an open pipe held by the parent it hangs before producing any output.

const chunks = [];

process.stdin.on("data", (chunk) => {
  chunks.push(chunk);
});

process.stdin.on("end", () => {
  const input = Buffer.concat(chunks);
  process.stdout.write(`stdin bytes=${input.length}\n`);
  process.stdout.write("started\n");
});

process.stdin.resume();
