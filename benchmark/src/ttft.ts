import cp from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { setup, TURBO_BIN, DEFAULT_EXEC_OPTS } from "./helpers";

const profileName = process.argv[2];

if (!profileName) {
  console.error("Error: Missing profile name");
  printUsageMessage();
  process.exit(1);
}

const fullProfilePath = path.join(process.cwd(), profileName);

if (fs.existsSync(fullProfilePath)) {
  console.error(`Error: ${fullProfilePath} already exists`);
  printUsageMessage();
  process.exit(1);
}

console.log(`Saving profile to ${fullProfilePath}`);

cp.execSync(`${TURBO_BIN} --version`, { stdio: "inherit" });

// Sets up the monorepo
setup();

if (!fs.existsSync(TURBO_BIN)) {
  throw new Error("No turbo binary found");
}

const turboFlags = `-vvv --dry --skip-infer --profile=${fullProfilePath}`;

console.log("Executing turbo build in child process", {
  cwd: process.cwd(),
  bin: TURBO_BIN,
  execOpts: DEFAULT_EXEC_OPTS,
  turboFlags,
});

// When this script runs, cwd is benchmark/large-monorepo (i.e. REPO_PATH)
const cmd = `${TURBO_BIN} run build ${turboFlags}`;
try {
  cp.execSync(cmd, {
    ...DEFAULT_EXEC_OPTS,
    env: { ...process.env, EXPERIMENTAL_RUST_CODEPATH: "true" },
  });
} catch (e) {
  // catch errors and exit. the build command seems to be erroring out due to very large output?
  // need to chase it down, but the benchmark seems to still be working, and when the same turbo run build
  // is executed _without_ a child process, it works and has a 0 exit code.
  console.error("Error running turbo build", e);
}

// -----------------------
// Helpers
// -----------------------

function printUsageMessage() {
  console.log("Usage:\n\npnpm -F benchmark ttft <path>");
}
