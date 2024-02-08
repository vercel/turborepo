import * as path from "node:path";
import { Workspace, Package, PackageManager } from "../js/dist/index.js";

interface AffectedPackagesTestParams {
  files: string[];
  expected: string[];
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
        files: ["apps/app/file.txt"],
        expected: ["app-a"],
        description: "app change",
      },
      {
        files: ["packages/ui/a.txt"],
        expected: ["app-a", "ui"],
        description: "lib change",
      },
      {
        files: ["package.json"],
        expected: ["app-a", "ui"],
        description: "global change",
      },
      {
        files: ["README.md"],
        expected: [],
        description: "global change that can be ignored",
      },
    ];

    test.each(tests)(
      "$description",
      async (testParams: AffectedPackagesTestParams) => {
        const { files, expected } = testParams;
        const dir = path.resolve(__dirname, "./fixtures/monorepo");
        const workspace = await Workspace.find(dir);
        const changedPackages = await workspace.changedPackages(files);
        expect(changedPackages).toEqual(expected);
      }
    );
  });
});
