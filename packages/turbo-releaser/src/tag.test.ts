import assert from "node:assert/strict";
import path from "node:path";
import { describe, it, mock } from "node:test";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { createReleaseTag } from "./tag";

describe("createReleaseTag", () => {
  it("creates and pushes a missing tag", async () => {
    const root = path.join(tmpdir(), "turbo-tag-test");
    await rm(root, { recursive: true, force: true });
    await mkdir(root, { recursive: true });
    await writeFile(path.join(root, "version.txt"), "1.2.3\nlatest\n");
    const run = mock.fn(
      (_command: string, _args: Array<string>, _cwd: string) => undefined
    );

    await createReleaseTag({
      repoRoot: root,
      versionPath: "version.txt",
      dependencies: {
        run,
        capture: mock.fn(
          (_command: string, args: Array<string>, _cwd: string) =>
            args[0] === "rev-parse" ? "local-sha\n" : ""
        )
      }
    });

    assert.deepEqual(
      run.mock.calls.map(({ arguments: args }) => args.slice(0, 2)),
      [
        ["git", ["tag", "v1.2.3"]],
        ["git", ["push", "origin", "v1.2.3"]]
      ]
    );
  });

  it("does nothing when the remote tag is correct", async () => {
    const root = path.join(tmpdir(), "turbo-tag-existing-test");
    await rm(root, { recursive: true, force: true });
    await mkdir(root, { recursive: true });
    await writeFile(path.join(root, "version.txt"), "1.2.3\nlatest\n");
    const run = mock.fn(
      (_command: string, _args: Array<string>, _cwd: string) => undefined
    );

    await createReleaseTag({
      repoRoot: root,
      versionPath: "version.txt",
      dependencies: {
        run,
        capture: mock.fn(() => "same-sha\n")
      }
    });

    assert.equal(run.mock.callCount(), 0);
  });
});
