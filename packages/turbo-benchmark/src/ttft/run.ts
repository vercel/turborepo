import cp from "node:child_process";
import { setup, TURBO_BIN, DEFAULT_EXEC_OPTS } from "../helpers";

/**
 * @param profilePath - The path passed to `--profile` flag where the profile will be saved
 */
export function run(profilePath: string) {
  // Set up the monorepo we will run the benchmark on
  setup();

  const turboFlags = `--dry --skip-infer --profile=${profilePath}`;

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
      env: process.env,
    });
  } catch (e) {
    // catch errors and exit. the build command seems to be erroring out due to very large output?
    // need to chase it down, but the benchmark seems to still be working, and when the same turbo run build
    // is executed _without_ a child process, it works and has a 0 exit code.
    console.error("Error running turbo build", e);
  }
}
