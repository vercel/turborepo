// @ts-ignore-next-line
import * as uvu from "uvu";
import path from "path";
import * as assert from "uvu/assert";
import {
  getCommandOutputAsArray,
  getHashFromOutput,
  getCacheItemForHash,
  getCachedLogFilePathForTask,
  extractZst,
} from "../helpers";
import { Monorepo } from "../monorepo";

export default function (
  suite: uvu.uvu.Test<uvu.Context>,
  repo: Monorepo,
  pkgManager: string,
  options?: { cwd?: string }
) {
  return suite(`${pkgManager} runs tests and logs`, async () => {
    const results = repo.turbo("run", ["test"], options);
    assert.equal(0, results.exitCode, "exit code should be 0");
    const commandOutput = getCommandOutputAsArray(results);
    const hash = getHashFromOutput(commandOutput, "c#test");
    assert.ok(!!hash, "No hash for c#test");

    const cacheItemPath = getCacheItemForHash(repo, hash);
    await extractZst(path.join(repo.root, cacheItemPath), repo.root);

    const cachedLogFilePath = getCachedLogFilePathForTask(
      repo,
      path.join("packages", "c"),
      "test"
    );
    let text = "";
    assert.not.throws(() => {
      text = repo.readFileSync(cachedLogFilePath);
    }, `Could not read cached log file from cache ${cachedLogFilePath}`);
    assert.ok(text.includes("testing c"), "Contains correct output");
  });
}
