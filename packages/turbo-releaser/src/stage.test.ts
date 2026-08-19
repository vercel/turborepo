import assert from "node:assert/strict";
import path from "node:path";
import { describe, it, mock } from "node:test";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { releasePackages } from "./config";
import { prepareStage } from "./stage";

describe("prepareStage", () => {
  it("updates packages and skill references before creating the branch", async () => {
    const root = path.join(tmpdir(), "turbo-stage-test");
    const skillRoot = path.join(root, "skills", "turborepo");
    await rm(root, { recursive: true, force: true });
    await mkdir(path.join(skillRoot, "references"), { recursive: true });
    await writeFile(path.join(root, "version.txt"), "1.2.3\ncanary\n");
    await writeFile(
      path.join(skillRoot, "SKILL.md"),
      "---\nmetadata:\n  version: 1.2.2\n---\nhttps://turborepo.dev/schema.json\n"
    );
    await writeFile(
      path.join(skillRoot, "references", "guide.md"),
      "https://turbo.build/schema.v2.json\n"
    );

    const run = mock.fn(
      (_command: string, _args: Array<string>, _options: object) => undefined
    );
    const capture = mock.fn(
      (_command: string, args: Array<string>, _cwd: string) =>
        args[0] === "diff" ? "version changed" : ""
    );

    const result = await prepareStage({
      repoRoot: root,
      versionPath: "version.txt",
      dependencies: { run, capture }
    });

    assert.deepEqual(result, { branch: "staging-1.2.3", version: "1.2.3" });
    const versionCalls = run.mock.calls.filter(
      ({ arguments: args }) => args[0] === "pnpm"
    );
    assert.equal(versionCalls.length, releasePackages.length);
    assert.deepEqual(versionCalls[0]?.arguments[1], [
      "version",
      "1.2.3",
      "--allow-same-version",
      "--no-git-tag-version"
    ]);
    assert.match(
      await readFile(path.join(skillRoot, "SKILL.md"), "utf8"),
      /version: 1\.2\.3/
    );
    assert.match(
      await readFile(path.join(skillRoot, "references", "guide.md"), "utf8"),
      /https:\/\/v1-2-3\.turborepo\.dev\/schema\.json/
    );
    assert.deepEqual(run.mock.calls.at(-1)?.arguments.slice(0, 2), [
      "git",
      ["checkout", "-b", "staging-1.2.3"]
    ]);
  });

  it("rejects an existing staging branch", async () => {
    const root = path.join(tmpdir(), "turbo-stage-existing-test");
    await rm(root, { recursive: true, force: true });
    await mkdir(root, { recursive: true });
    await writeFile(path.join(root, "version.txt"), "1.2.3\nlatest\n");

    await assert.rejects(
      prepareStage({
        repoRoot: root,
        versionPath: "version.txt",
        dependencies: {
          run: mock.fn(
            (_command: string, _args: Array<string>, _options: object) =>
              undefined
          ),
          capture: mock.fn(
            (_command: string, args: Array<string>, _cwd: string) => {
              if (args[0] === "diff") {
                return "version changed";
              }
              if (args[0] === "ls-remote" && args[1] === "--heads") {
                return "existing sha";
              }
              return "";
            }
          )
        }
      }),
      /Staging branch staging-1\.2\.3 already exists/
    );
  });
});
