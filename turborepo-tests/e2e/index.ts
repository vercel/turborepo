import execa from "execa";
import * as uvu from "uvu";
import { Monorepo, createMonorepo } from "./monorepo";
import path from "path";
import { basicPipeline } from "./fixtures";
import type { PackageManager } from "./types";
import pruneTests from "./prune-test";

import testBuild from "./tests/builds";
import testBuild from "./tests/builds";
import testsAndLogs from "./tests/tests-and-logs";
import lintAndLogs from "./tests/lint-and-logs";
import changes from "./tests/changes";
import rootTasks from "./tests/root-tasks";
import passThroughArgs from "./tests/passthru-args";
import prune from "./tests/prune";
import pruneDocker from "./tests/prune-docker";

const testCombinations = [
  { pkgManager: "yarn" as PackageManager, pipeline: basicPipeline },
  { pkgManager: "berry" as PackageManager, pipeline: basicPipeline },
  { pkgManager: "pnpm6" as PackageManager, pipeline: basicPipeline },
  { pkgManager: "pnpm" as PackageManager, pipeline: basicPipeline },
  { pkgManager: "npm" as PackageManager, pipeline: basicPipeline },
];

// This is injected by github actions
process.env.TURBO_TOKEN = "";

let suites: uvu.uvu.Test<uvu.Context>[] = [];

for (const combo of testCombinations) {
  const { pkgManager, pipeline } = combo;

  // Run all the tests from the root of the repo
  const Basic = uvu.suite(pkgManager);
  const repo = createMonorepo(pkgManager, pipeline);
  repo.expectCleanGitStatus();
  testBuild(Basic, repo, pkgManager);
  testsAndLogs(Basic, repo, pkgManager);
  lintAndLogs(Basic, repo, pkgManager);
  changes(Basic, repo, pkgManager);
  rootTasks(Basic, repo, pkgManager);
  passThroughArgs(Basic, repo, pkgManager);
  prune(Basic, repo, pkgManager);
  pruneDocker(Basic, repo, pkgManager);

  // test that turbo can run from a subdirectory
  const BasicFromSubDir = uvu.suite(`${pkgManager} from subdirectory`);
  const repo2 = createMonorepo(pkgManager, pipeline, "js");
  const cwd = path.join(repo2.root, repo2.subdir);
  testBuild(BasicFromSubDir, repo2, pkgManager, { cwd });
  testsAndLogs(BasicFromSubDir, repo2, pkgManager, { cwd });
  lintAndLogs(BasicFromSubDir, repo2, pkgManager, { cwd });
  changes(BasicFromSubDir, repo2, pkgManager, { cwd });
  rootTasks(BasicFromSubDir, repo2, pkgManager, { cwd });
  passThroughArgs(BasicFromSubDir, repo2, pkgManager, { cwd });
  prune(BasicFromSubDir, repo2, pkgManager, { cwd });
  pruneDocker(BasicFromSubDir, repo2, pkgManager, { cwd });

  Basic.after(() => repo.cleanup());
  BasicFromSubDir.after(() => repo2.cleanup());
  Basic.run();
  BasicFromSubDir.run();
}

pruneTests();
