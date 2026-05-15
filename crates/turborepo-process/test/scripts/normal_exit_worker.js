const { spawn } = require("node:child_process");
const fs = require("node:fs");

const mode = process.argv[2];

if (mode === "worker") {
  const markerPath = process.argv[3];
  const delayMs = Number(process.argv[4]);

  setTimeout(() => {
    fs.writeFileSync(markerPath, "done\n");
  }, delayMs);
} else {
  const markerPath = process.argv[2];
  const pidPath = process.argv[3];
  const delayMs = process.argv[4];
  const worker = spawn(process.execPath, [__filename, "worker", markerPath, delayMs], {
    detached: process.platform === "win32",
    stdio: "ignore",
  });

  fs.writeFileSync(pidPath, `${worker.pid}\n`);
  worker.unref();
}
