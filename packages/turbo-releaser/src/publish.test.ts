import assert from "node:assert/strict";
import path from "node:path";
import { describe, it, mock } from "node:test";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { releasePackages, supportedPlatforms } from "./config";
import { publishRelease } from "./publish";
import type { packAndPublish } from "./packager";
import type { publishWithRetries } from "./npm";

describe("publishRelease", () => {
  it("builds and packs every package during a dry run", async () => {
    const root = path.join(tmpdir(), "turbo-releaser-publish-test");
    await rm(root, { recursive: true, force: true });
    await mkdir(root, { recursive: true });
    await writeFile(path.join(root, "version.txt"), "1.2.3\ncanary\n");

    const run = mock.fn(
      (_command: string, _args: Array<string>, _options: object) => undefined
    );
    const pack = mock.fn((_options: Parameters<typeof packAndPublish>[0]) =>
      Promise.resolve()
    );
    const publish = mock.fn(
      (_options: Parameters<typeof publishWithRetries>[0]) => Promise.resolve()
    );

    await publishRelease({
      repoRoot: root,
      artifactsDir: "cli",
      versionPath: "version.txt",
      skipPublish: true,
      dependencies: {
        run,
        packAndPublish: pack,
        publishWithRetries: publish
      }
    });

    assert.deepEqual(run.mock.calls[0].arguments.slice(0, 2), [
      "turbo",
      [
        "run",
        "build",
        "copy-schema",
        "--filter=create-turbo",
        "--filter=@turbo/codemod",
        "--filter=turbo-ignore",
        "--filter=@turbo/workspaces",
        "--filter=@turbo/gen",
        "--filter=eslint-plugin-turbo",
        "--filter=eslint-config-turbo",
        "--filter=@turbo/types"
      ]
    ]);
    assert.equal(pack.mock.callCount(), 1);
    assert.equal(
      pack.mock.calls[0].arguments[0].srcDir,
      path.join(root, "cli")
    );
    assert.deepEqual(
      pack.mock.calls[0].arguments[0].platforms,
      supportedPlatforms
    );
    assert.equal(run.mock.callCount(), releasePackages.length + 2);
    assert.equal(publish.mock.callCount(), 0);
  });

  it("publishes JavaScript packages in configured order", async () => {
    const root = path.join(tmpdir(), "turbo-releaser-publish-order-test");
    await rm(root, { recursive: true, force: true });
    await mkdir(root, { recursive: true });
    await writeFile(path.join(root, "version.txt"), "1.2.3\nlatest\n");

    const run = mock.fn((command: string, args: Array<string>) => {
      if (command === "npm" && args[0] === "view") {
        throw new Error("not found");
      }
    });
    const publish = mock.fn(
      (_options: Parameters<typeof publishWithRetries>[0]) => Promise.resolve()
    );

    await publishRelease({
      repoRoot: root,
      artifactsDir: "cli",
      versionPath: "version.txt",
      skipPublish: false,
      dependencies: {
        run,
        packAndPublish: mock.fn(
          (_options: Parameters<typeof packAndPublish>[0]) => Promise.resolve()
        ),
        publishWithRetries: publish
      }
    });

    assert.deepEqual(
      publish.mock.calls.map(({ arguments: args }) => args[0].packageName),
      releasePackages.map(({ name }) => `${name}@1.2.3`)
    );
  });

  it("refuses to publish an existing turbo version", async () => {
    const root = path.join(tmpdir(), "turbo-releaser-existing-test");
    await rm(root, { recursive: true, force: true });
    await mkdir(root, { recursive: true });
    await writeFile(path.join(root, "version.txt"), "1.2.3\nlatest\n");

    await assert.rejects(
      publishRelease({
        repoRoot: root,
        artifactsDir: "cli",
        versionPath: "version.txt",
        skipPublish: false,
        dependencies: {
          run: mock.fn(
            (_command: string, _args: Array<string>, _options: object) =>
              undefined
          ),
          packAndPublish: mock.fn(
            (_options: Parameters<typeof packAndPublish>[0]) =>
              Promise.resolve()
          ),
          publishWithRetries: mock.fn(
            (_options: Parameters<typeof publishWithRetries>[0]) =>
              Promise.resolve()
          )
        }
      }),
      /turbo@1\.2\.3 already exists/
    );
  });
});
