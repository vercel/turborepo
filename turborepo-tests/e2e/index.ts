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
  const cwd = path.join(repo2.root, repo2.subdir ? repo2.subdir : ""); // We know repo2 always has a subdir, but typescript doesn't
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
