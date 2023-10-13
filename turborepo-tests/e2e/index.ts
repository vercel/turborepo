import execa from "execa";
import * as uvu from "uvu";
import { Monorepo, createMonorepo } from "./monorepo";
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

const packageManagers: PackageManager[] = [
  "yarn",
  "berry",
  "pnpm6",
  "pnpm",
  "npm",
];

// This is injected by github actions
process.env.TURBO_TOKEN = "";

let suites: uvu.uvu.Test<uvu.Context>[] = [];

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
  prune(Basic, repo, mgr);
  pruneDocker(Basic, repo, mgr);

  // test that turbo can run from a subdirectory
  const BasicFromSubDir = uvu.suite(`${mgr} from subdirectory`);
  const repo2 = createMonorepo(
    `${mgr}-in-subdirectory`,
    mgr,
    basicPipeline,
    "js"
  );
  const cwd = path.join(repo2.root, repo2.subdir);
  testBuild(BasicFromSubDir, repo2, mgr, { cwd });
  testsAndLogs(BasicFromSubDir, repo2, mgr, { cwd });
  lintAndLogs(BasicFromSubDir, repo2, mgr, { cwd });
  changes(BasicFromSubDir, repo2, mgr, { cwd });
  rootTasks(BasicFromSubDir, repo2, mgr, { cwd });
  passThroughArgs(BasicFromSubDir, repo2, mgr, { cwd });
  prune(BasicFromSubDir, repo2, mgr, { cwd });
  pruneDocker(BasicFromSubDir, repo2, mgr, { cwd });

  Basic.after(() => repo.cleanup());
  BasicFromSubDir.after(() => repo2.cleanup());
  Basic.run();
  BasicFromSubDir.run();
}

////// More explicit prune tests ///////////////////

// prune and prune --docker with a
// expect c#build to be removed, since there is no dep between a -> c
const Prune = uvu.suite("basicPrune");
const repo = createMonorepo("yarn", prunePipeline);
prune(Prune, repo, "yarn", {}, ["a#build"], ["c#build"]);
pruneDocker(Prune, repo, "yarn", {}, ["a#build"], ["c#build"]);

// prune and prune --docker from subdir
// expect c#build to be removed, since there is no dep between a -> c
const PruneFromSubDir = uvu.suite("basicPrune from subdirectory");
const repo1 = createMonorepo("yarn", prunePipeline, "js");
const cwd = path.join(repo1.root, repo1.subdir);
prune(PruneFromSubDir, repo, "yarn", { cwd }, ["a#build"], ["c#build"]);
pruneDocker(PruneFromSubDir, repo1, "yarn", { cwd }, ["a#build"], ["c#build"]);

////////////////// Explicit Deps Prune //////////////////

// prune and prune --docker
// expect c#build to be included, since a depends on c
const ExplicitDepPrune = uvu.suite("explicitDepPrune");
const repo2 = createMonorepo("yarn", explicitPrunePipeline);
prune(ExplicitDepPrune, repo2, "yarn", {}, ["a#build", "b#build"], ["c#build"]);
pruneDocker(
  ExplicitDepPrune,
  repo2,
  "yarn",
  {},
  ["a#build", "b#build"],
  ["c#build"]
);

// prune and prune --docker from subdir
// expect b#build to be included, since a depends on b
const ExplicitDepPruneFromSubDir = uvu.suite(
  "explicitDepPrune from subdirectory"
);
const repo3 = createMonorepo("yarn", explicitPrunePipeline, "js");
const repo3cwd = path.join(repo3.root, repo3.subdir);
prune(
  ExplicitDepPruneFromSubDir,
  repo,
  "yarn",
  { cwd: repo3cwd },
  ["a#build", "b#build"],
  ["c#build"]
);
pruneDocker(
  ExplicitDepPruneFromSubDir,
  repo3,
  "yarn",
  { cwd: repo3cwd },
  ["a#build", "b#build"],
  ["c#build"]
);

// Setup the cleanup in after hooks for each suite
Prune.after(() => repo.cleanup());
PruneFromSubDir.after(() => repo1.cleanup());
ExplicitDepPrune.after(() => repo2.cleanup());
ExplicitDepPruneFromSubDir.after(() => repo3.cleanup());

// Actually execute all the tests
Prune.run();
PruneFromSubDir.run();
ExplicitDepPrune.run();
ExplicitDepPruneFromSubDir.run();
