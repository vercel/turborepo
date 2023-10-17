import cp from "child_process";
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

  console.log(
    "Executing turbo build in child process with opts",
    DEFAULT_EXEC_OPTS
  );

  // Path to profile.json is ../profile.json because we are in
  // benchmark/large-monorepo (i.e. REPO_PATH) when this runs
  cp.execSync(
    `${TURBO_BIN} run build -vvv --experimental-rust-codepath --dry --skip-infer --profile=../profile.json`,
    DEFAULT_EXEC_OPTS
  );

  // TODO: just do this in JS and send to TB here instead of child process?
  cp.execSync("jq -f src/fold.jq < profile.json > ttft.json", {
    stdio: "inherit",
  });
})();
