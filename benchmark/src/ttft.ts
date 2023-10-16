import cp, { StdioOptions } from "child_process";
import fs from "fs";

import { setup, TURBO_BIN, DEFAULT_EXEC_OPTS, REPO_PATH } from "./helpers";

(async function () {
  cp.execSync(`${TURBO_BIN} --version`, { stdio: "inherit" });

  console.log("setup");
  setup();

  if (!fs.existsSync(TURBO_BIN)) {
    throw new Error("No turbo binary found");
  }

  console.log("running ttft", {
    cwd: process.cwd(),
    bin: TURBO_BIN,
  });

  // Path to profile.json is ../profile.json because we are in
  // benchmark/large-monorepo (i.e. REPO_PATH) when this runs
  const opts = {
    ...DEFAULT_EXEC_OPTS,
    stdio: "inherit" as StdioOptions,
    env: {
      TURBO_LOG_VERBOSITY: "trace",
      EXPERIMENTAL_RUST_CODEPATH: "true",
    },
  };
  console.log("Executing turbo build in child process with opts", opts);

  try {
    cp.execSync(
      `${TURBO_BIN} run build --skip-infer --force --dry --profile ../profile.json`,
      opts
    );
  } catch (e) {
    // not sure why this is erroring
  }

  // TODO: just do this in JS and send to TB here instead of child process?
  // TODO: write this to ttft.json
  const out = cp.execSync("jq -f src/fold.jq < profile.json > ttft.json", {
    stdio: "inherit",
  });
})();
