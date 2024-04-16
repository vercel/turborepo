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
    assert.equal(packageManager.name, "pnpm");
  });
});
