// This intentionally blocks until stdin reaches EOF. A non-persistent task
// should see stdin closed immediately by Turbo; an open pipe causes a hang
// before any startup output is produced.
const chunks = [];

process.stdin.on("data", (chunk) => {
  chunks.push(chunk);
});

process.stdin.on("end", () => {
  const input = Buffer.concat(chunks);
  console.log(`stdin bytes=${input.length}`);
  console.log("started");
});

process.stdin.resume();
