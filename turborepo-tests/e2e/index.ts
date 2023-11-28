import * as uvu from "uvu";
import { createMonorepo } from "./monorepo";
// @ts-ignore-next-line
import path from "path";
import { basicPipeline } from "./fixtures";
import type { PackageManager } from "./types";

import testBuild from "./tests/builds";
import testsAndLogs from "./tests/tests-and-logs";
import lintAndLogs from "./tests/lint-and-logs";
import changes from "./tests/changes";
import rootTasks from "./tests/root-tasks";
import passThroughArgs from "./tests/passthru-args";

const packageManagers: PackageManager[] = [
  "yarn",
  "berry",
  "pnpm6",
  "pnpm",
  "npm",
];

// This is injected by github actions
process.env.TURBO_TOKEN = "";

for (const mgr of packageManagers) {
  // Run all the tests from the root of the repo
  const Basic = uvu.suite(mgr);
  const repo = createMonorepo(`${mgr}-basic`, mgr, basicPipeline);
  repo.expectCleanGitStatus();
  testBuild(Basic, repo, mgr);
  testsAndLogs(Basic, repo, mgr);
  lintAndLogs(Basic, repo, mgr);
  changes(Basic, repo, mgr);
  rootTasks(Basic, repo, mgr);
  passThroughArgs(Basic, repo, mgr);

  Basic.after(() => repo.cleanup());
  Basic.run();
}
