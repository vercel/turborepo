import { describe, it } from "node:test";
import { strict as assert } from "node:assert";
import * as path from "node:path";
import { Workspace, Package, PackageManager } from "../js/dist/index.js";

type PackageReduced = Pick<Package, "name" | "relativePath">;

interface AffectedPackagesTestParams {
  description: string;
  files: string[];
  changedLockfile?: string | undefined | null;
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
      description:
        "a lockfile change will only affect packages impacted by the change",
      files: [],
      changedLockfile: `lockfileVersion: '6.0'

settings:
  autoInstallPeers: true
  excludeLinksFromLockfile: false

importers:

  .: {}

  apps/app:
    dependencies:
      microdiff:
        specifier: ^1.4.0
        version: 1.5.0
      ui:
        specifier: workspace:*
        version: link:../../packages/ui

  packages/blank: {}

  packages/ui: {}

packages:

  /microdiff@1.5.0:
    resolution: {integrity: sha512-Drq+/THMvDdzRYrK0oxJmOKiC24ayUV8ahrt8l3oRK51PWt6gdtrIGrlIH3pT/lFh1z93FbAcidtsHcWbnRz8Q==}
    dev: false
`,
      expected: [{ name: "app-a", relativePath: "apps/app" }],
    },
  ];

  for (const { description, files, expected, changedLockfile } of tests) {
    it(description, async () => {
      const dir = path.resolve(__dirname, "./fixtures/monorepo");
      const workspace = await Workspace.find(dir);

      const reduced: PackageReduced[] = (
        await workspace.affectedPackages(files, changedLockfile)
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
