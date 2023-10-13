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
  const { pkgManager, pipeline, name } = combo;

  const suiteNamePrefix = `${pkgManager}${name ? ": " + name : ""}`;

  const Suite = uvu.suite(suiteNamePrefix);
  const SubDirSuite = uvu.suite(`${suiteNamePrefix} from subdirectory`);

  const repo = createMonorepo(pkgManager, pipeline);
  repo.expectCleanGitStatus();
  runSmokeTests(Suite, repo, pkgManager);

  // test that turbo can run from a subdirectory
  const repo2 = createMonorepo(pkgManager, pipeline, "js");
  runSmokeTests(SubDirSuite, repo2, pkgManager, {
    cwd: path.join(repo2.root, repo2.subdir),
  });

  suites.push(Suite);
  suites.push(SubDirSuite);
}

for (let suite of suites) {
  suite.run();
}

function runSmokeTests<T>(
  suite: uvu.Test<T>,
  repo: Monorepo,
  pkgManager: PackageManager,
  options: execa.SyncOptions<string> = {}
) {
  suite.after(() => {
    repo.cleanup();
  });

  testBuild(suite, repo, pkgManager, options);
  testsAndLogs(suite, repo, pkgManager, options);
  lintAndLogs(suite, repo, pkgManager, options);
  changes(suite, repo, pkgManager, options);
  rootTasks(suite, repo, pkgManager, options);
  passThroughArgs(suite, repo, pkgManager, options);
  prune(suite, repo, pkgManager, options);
  pruneDocker(suite, repo, pkgManager, options);
}

pruneTests();
