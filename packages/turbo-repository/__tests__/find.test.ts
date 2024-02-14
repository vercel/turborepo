import * as path from "node:path";
import { Workspace, Package, PackageManager } from "../js/dist/index.js";

type PackageReduced = Pick<Package, "name" | "relativePath">;

interface AffectedPackagesTestParams {
  files: string[];
  expected: PackageReduced[];
  description: string;
}

describe("Workspace", () => {
  it("finds a workspace", async () => {
    const workspace = await Workspace.find();
    const expectedRoot = path.resolve(__dirname, "../../..");
    expect(workspace.absolutePath).toBe(expectedRoot);
  });

  it("enumerates packages", async () => {
    const workspace = await Workspace.find();
    const packages: Package[] = await workspace.findPackages();
    expect(packages.length).not.toBe(0);
  });

  it("finds a package manager", async () => {
    const workspace = await Workspace.find();
    const packageManager: PackageManager = workspace.packageManager;
    expect(packageManager.name).toBe("pnpm");
  });

  test("returns a package graph", async () => {
    const dir = path.resolve(__dirname, "./fixtures/monorepo");
    const workspace = await Workspace.find(dir);
    const graph = await workspace.findPackagesAndDependents();
    expect(graph).toEqual({
      "apps/app": [],
      "packages/ui": ["apps/app"],
    });
  });

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
        description: "global change that can be ignored",
        files: ["README.md"],
        expected: [],
      },
    ];

    test.each(tests)(
      "$description",
      async (testParams: AffectedPackagesTestParams) => {
        const { files, expected } = testParams;
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

        expect(reduced).toEqual(expected);
      }
    );
  });
});
