import cp from "node:child_process";
import fs from "node:fs";
import { setup, TURBO_BIN, DEFAULT_EXEC_OPTS } from "./helpers";

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
try {
  cp.execSync(
    `${TURBO_BIN} run build -vvv --experimental-rust-codepath --dry --skip-infer --profile=../profile.json`,
    DEFAULT_EXEC_OPTS
  );
} catch (e) {
  // catch errors and exit. the build command seems to be erroring out due to very large output?
  // need to chase it down, but the benchmark seems to still be working, and when the same turbo run build
  // is executed _without_ a child process, it works and has a 0 exit code.
  console.error("Error running turbo build", e);
}

// TODO: just do this in JS and send to TB here instead of child process?
cp.execSync("jq -f src/fold.jq < profile.json > ttft.json", {
  stdio: "inherit",
});
