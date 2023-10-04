const core = require("@actions/core");
const exec = require("@actions/exec");

async function runSweep(...args) {
  // TODO(alexkirsz) A cargo change introduced a regression where cargo can't
  // find the sweep binary. This is a temporary workaround until the fix is
  // released. See:
  // https://github.com/rust-lang/cargo/pull/11814
  await exec.exec("cargo-sweep", ["sweep", ...args]);
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
