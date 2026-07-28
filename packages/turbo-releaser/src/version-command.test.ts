import assert from "node:assert/strict";
import path from "node:path";
import { describe, it } from "node:test";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { updateVersion } from "./version-command";

describe("updateVersion", () => {
  it("increments canary versions", async () => {
    const directory = path.join(tmpdir(), "turbo-version-command-test");
    const versionPath = path.join(directory, "version.txt");
    await rm(directory, { recursive: true, force: true });
    await mkdir(directory, { recursive: true });
    await writeFile(versionPath, "1.2.3-canary.0\ncanary\n");

    const result = await updateVersion({
      versionPath,
      increment: "prerelease"
    });

    assert.deepEqual(result, { version: "1.2.3-canary.1", npmTag: "canary" });
    assert.equal(
      await readFile(versionPath, "utf8"),
      "1.2.3-canary.1\ncanary\n"
    );
  });

  it("applies a dist-tag override", async () => {
    const directory = path.join(tmpdir(), "turbo-version-override-test");
    const versionPath = path.join(directory, "version.txt");
    await rm(directory, { recursive: true, force: true });
    await mkdir(directory, { recursive: true });
    await writeFile(versionPath, "1.2.3\nlatest\n");

    const result = await updateVersion({
      versionPath,
      increment: "patch",
      tagOverride: "backport"
    });

    assert.deepEqual(result, { version: "1.2.4", npmTag: "backport" });
  });
});
