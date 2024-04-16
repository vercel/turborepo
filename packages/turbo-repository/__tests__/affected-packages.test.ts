import { describe, test } from "node:test";
import { strict as assert } from "node:assert";
import * as path from "node:path";
import { Workspace, Package, PackageManager } from "../js/dist/index.js";

type PackageReduced = Pick<Package, "name" | "relativePath">;

interface AffectedPackagesTestParams {
  files: string[];
  expected: PackageReduced[];
  description: string;
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
      description:
        "global change should be irrelevant but still triggers all packages",
      files: ["README.md"],
      expected: [
        { name: "app-a", relativePath: "apps/app" },
        { name: "ui", relativePath: "packages/ui" },
      ],
    },
  ];

  for (const { description, files, expected } of tests) {
    test(description, async () => {
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
});
