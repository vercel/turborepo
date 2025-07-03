import { beforeEach, describe, it } from "node:test";
import { strict as assert } from "node:assert";
import * as path from "node:path";
import { Workspace, Package, PackageManager } from "../js/dist/index.js";

type PackageReduced = Pick<Package, "name" | "relativePath">;

interface AffectedPackagesTestParams {
  description: string;
  files: string[];
  expected: PackageReduced[];
}

describe("affectedPackages", () => {
  const tests: AffectedPackagesTestParams[] = [
    {
      description: "app change",
      files: ["apps/app/file.txt"],
      expected: [{ name: "app-a", relativePath: "apps/app" }],
    },
    {
      description: "lib change",
      files: ["packages/ui/a.txt"],
      expected: [{ name: "ui", relativePath: "packages/ui" }],
    },
    {
      description: "global change",
      files: ["package.json"],
      expected: [
        { name: "app-a", relativePath: "apps/app" },
        { name: "ui", relativePath: "packages/ui" },
      ],
    },
    {
      description: "a lockfile change will affect all packages",
      files: ["pnpm-lock.yaml"],
      expected: [
        { name: "app-a", relativePath: "apps/app" },
        { name: "ui", relativePath: "packages/ui" },
      ],
    },
  ];

  for (const { description, files, expected } of tests) {
    it(description, async () => {
      const dir = path.resolve(__dirname, "./fixtures/monorepo");
      const workspace = await Workspace.find(dir);

      const reduced: PackageReduced[] = (
        await workspace.affectedPackages(files)
      ).map((pkg) => {
        return {
          name: pkg.name,
          relativePath: pkg.relativePath,
        };
      });

      assert.deepEqual(reduced, expected);
    });
  }

  it("does not require packageManager for npm", async () => {
    const dir = path.resolve(__dirname, "./fixtures/npm-monorepo");
    const workspace = await Workspace.find(dir);

    const reduced: PackageReduced[] = (
      await workspace.affectedPackages(["apps/app/file.txt"])
    ).map((pkg) => {
      return {
        name: pkg.name,
        relativePath: pkg.relativePath,
      };
    });

    assert.deepEqual(reduced, [{ name: "app-a", relativePath: "apps/app" }]);
  });

  describe("optimizedLockfileUpdates", () => {
    it("errors if not provided comparison ref", async () => {
      const dir = path.resolve(__dirname, "./fixtures/monorepo");
      const workspace = await Workspace.find(dir);

      assert.rejects(
        workspace.affectedPackages(["pnpm-lock.yaml"], null, true)
      );
    });

    it("still considers root file changes as global", async () => {
      const dir = path.resolve(__dirname, "./fixtures/monorepo");
      const workspace = await Workspace.find(dir);

      const reduced: PackageReduced[] = (
        await workspace.affectedPackages(
          ["file-we-do-not-understand.txt"],
          "HEAD",
          true
        )
      ).map((pkg) => {
        return {
          name: pkg.name,
          relativePath: pkg.relativePath,
        };
      });

      assert.deepEqual(reduced, [
        { name: "app-a", relativePath: "apps/app" },
        { name: "ui", relativePath: "packages/ui" },
      ]);
    });
  });
});
