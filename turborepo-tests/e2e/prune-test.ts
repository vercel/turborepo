import path from "path";
import * as uvu from "uvu";
import { createMonorepo } from "./monorepo";
import prune from "./tests/prune";
import pruneDocker from "./tests/prune-docker";
import { prunePipeline, explicitPrunePipeline } from "./fixtures";

export default function () {
  // prune and prune --docker
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
  pruneDocker(
    PruneFromSubDir,
    repo1,
    "yarn",
    { cwd },
    ["a#build"],
    ["c#build"]
  );

  ////////////////// Explicit Dep Prune //////////////////

  // prune and prune --docker
  // expect c#build to be included, since a depends on c
  const ExplicitDepPrune = uvu.suite("explicitDepPrune");
  const repo2 = createMonorepo("yarn", explicitPrunePipeline);
  prune(
    ExplicitDepPrune,
    repo2,
    "yarn",
    {},
    ["a#build", "b#build"],
    ["c#build"]
  );
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
}
