const core = require("@actions/core");
const exec = require("@actions/exec");

async function runSweep(...args) {
  await exec.exec("cargo", ["sweep", ...args]);
}

async function storeTimestamp() {
  await core.group("Storing timestamp to compare later", () =>
    runSweep("--stamp")
  );

  core.info("Timestamp stored in `sweep.timestamp`");
}

async function sweep() {
  await core.group("Cleaning old build artifacts", () => runSweep("--file"));

  core.info("Removed old build artifacts.");
}

module.exports = {
  storeTimestamp,
  sweep,
};
