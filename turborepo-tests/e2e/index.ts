import execa from "execa";
import * as uvu from "uvu";
import { Monorepo } from "./monorepo";
import path from "path";
import {
  basicPipeline,
  prunePipeline,
  explicitPrunePipeline,
} from "./fixtures";
import type { PackageManager } from "./types";

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

  // there is probably no need to test every
  // pipeline against every package manager,
  // so specify directly rather than use the
  // cartesian product
  {
    pkgManager: "yarn" as PackageManager,
    pipeline: prunePipeline,
    name: "basicPrune",
    excludePrune: ["c#build"],
    includePrune: ["a#build"],
  }, // expect c#build to be removed, since there is no dep between a -> c
  {
    pkgManager: "yarn" as PackageManager,
    pipeline: explicitPrunePipeline,
    name: "explicitDepPrune",
    excludePrune: ["c#build"],
    includePrune: ["a#build", "b#build"],
  }, // expect c#build to be included, since a depends on c
];

// This is injected by github actions
process.env.TURBO_TOKEN = "";

let suites: uvu.uvu.Test<uvu.Context>[] = [];

for (const combo of testCombinations) {
  const {
    pkgManager,
    pipeline,
    name,
    includePrune = [],
    excludePrune = [],
  } = combo;

  const subdir = "js";
  const suiteNamePrefix = `${pkgManager}${name ? ": " + name : ""}`;

  const Suite = uvu.suite(suiteNamePrefix);
  const SubDirSuite = uvu.suite(`${suiteNamePrefix} from subdirectory`);

  const repo = new Monorepo({
    root: `${pkgManager}-basic`,
    pm: pkgManager,
    pipeline,
  });
  repo.init();
  repo.install();
  repo.addPackage("a", ["b"]);
  repo.addPackage("b");
  repo.addPackage("c");
  repo.linkPackages();
  repo.expectCleanGitStatus();
  runSmokeTests(Suite, repo, pkgManager, includePrune, excludePrune);

  // test that turbo can run from a subdirectory
  const sub = new Monorepo({
    root: `${pkgManager}-in-subdirectory`,
    pm: pkgManager,
    pipeline,
    subdir,
  });
  sub.init();
  sub.install();
  sub.addPackage("a", ["b"]);
  sub.addPackage("b");
  sub.addPackage("c");
  sub.linkPackages();

  runSmokeTests(SubDirSuite, sub, pkgManager, includePrune, excludePrune, {
    cwd: path.join(sub.root, sub.subdir),
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
  includePrune: string[],
  excludePrune: string[],
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
  prune(suite, repo, pkgManager, options, includePrune, excludePrune);
  pruneDocker(suite, repo, pkgManager, options, includePrune, excludePrune);
}
