import path from "path";
import * as assert from "uvu/assert";
import * as uvu from "uvu";
import {
  getCommandOutputAsArray,
  getHashFromOutput,
  getCacheItemForHash,
  getCachedLogFilePathForTask,
  extractZst,
} from "../helpers";
import { PackageManager } from "../types";
import { Monorepo } from "../monorepo";

export default function (
  suite: uvu.uvu.Test<uvu.Context>,
  repo: Monorepo,
  pkgManager: PackageManager,
  options: { cwd?: string } = {}
) {
  return suite(`${pkgManager} runs lint and logs`, async () => {
    const results = repo.turbo("run", ["lint"], options);
    assert.equal(0, results.exitCode, "exit code should be 0");
    const commandOutput = getCommandOutputAsArray(results);
    const hash = getHashFromOutput(commandOutput, "c#lint");
    assert.ok(!!hash, `No hash for c#lint in ${commandOutput.join("\n")}`);

    const cacheItemPath = getCacheItemForHash(repo, hash);
    await extractZst(path.join(repo.root, cacheItemPath), repo.root);

    const cachedLogFilePath = getCachedLogFilePathForTask(
      repo,
      path.join("packages", "c"),
      "lint"
    );
    let text = "";
    assert.not.throws(() => {
      text = repo.readFileSync(cachedLogFilePath);
    }, `Could not read cached log file from cache ${cachedLogFilePath}`);
    assert.ok(text.includes("linting c"), "Contains correct output");
  });
}
