import { describe, it } from "node:test";
import { strict as assert } from "node:assert";
import * as path from "node:path";
import { Workspace, PackageManager } from "../js/dist/index.js";

describe("Workspace", () => {
  it("finds a workspace", async () => {
    const workspace = await Workspace.find();
    const expectedRoot = path.resolve(__dirname, "../../..");
    assert.equal(workspace.absolutePath, expectedRoot);
  });

  it("finds a package manager", async () => {
    const workspace = await Workspace.find();
    const packageManager: PackageManager = workspace.packageManager;
    assert.equal(packageManager.name, "pnpm9");
  });

  // The CLI requires a declared package manager, but this library analyzes
  // repositories it doesn't control, so it must fall back to lockfile-based
  // detection when neither `packageManager` nor `devEngines.packageManager`
  // is present.
  it("detects the package manager from the lockfile when undeclared", async () => {
    const dir = path.resolve(__dirname, "./fixtures/npm-monorepo-no-pm");
    const workspace = await Workspace.find(dir);

    assert.equal(workspace.packageManager.name, "npm");
    assert.equal(workspace.isMultiPackage, true);

    const packages = await workspace.findPackages();
    assert.deepEqual(packages.map((pkg) => pkg.relativePath).sort(), [
      "apps/app",
      "packages/ui"
    ]);
  });
});
