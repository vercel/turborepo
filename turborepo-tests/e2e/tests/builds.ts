import * as uvu from "uvu";
import * as assert from "uvu/assert";
import {
  matchTask,
  includesTaskId,
  taskHashPredicate,
  getCommandOutputAsArray,
} from "../helpers";
import type { DryRun, PackageManager } from "../types";
import { Monorepo } from "../monorepo";

export default function (
  suite: uvu.uvu.Test<uvu.Context>,
  repo: Monorepo,
  pkgManager: PackageManager,
  options: { cwd?: string } = {}
) {
  return suite(`${pkgManager} builds`, async () => {
    const results = repo.turbo("run", ["build", "--dry=json"], options);
    const dryRun: DryRun = JSON.parse(results.stdout);
    // expect to run all packages
    const expectTaskId = includesTaskId(dryRun);
    for (const pkg of ["a", "b", "c", "//"]) {
      assert.ok(
        dryRun.packages.includes(pkg),
        `Expected to include package ${pkg}`
      );
      assert.ok(
        expectTaskId(pkg + "#build"),
        `Expected to include task ${pkg}#build`
      );
    }

    // actually run the build
    const buildOutput = getCommandOutputAsArray(
      repo.turbo("run", ["build"], options)
    );
    assert.ok(buildOutput.includes("//:build: building"), "Missing root build");

    // assert that hashes are stable across multiple runs
    const secondRun = repo.turbo("run", ["build", "--dry=json"], options);
    const secondDryRun: DryRun = JSON.parse(secondRun.stdout);

    repo.turbo("run", ["build"], options);

    const thirdRun = repo.turbo("run", ["build", "--dry=json"], options);
    const thirdDryRun: DryRun = JSON.parse(thirdRun.stdout);
    const getThirdRunHash = matchTask(taskHashPredicate)(thirdDryRun);
    for (const entry of secondDryRun.tasks) {
      assert.equal(
        getThirdRunHash(entry.taskId),
        entry.hash,
        `Hashes for ${entry.taskId} did not match`
      );
    }
  });
}
