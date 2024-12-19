import { describe, it } from "node:test";
import { strict as assert } from "node:assert";
import * as path from "node:path";
import { Workspace, Package } from "../js/dist/index.js";

describe("findPackages", () => {
  it("enumerates packages", async () => {
    const workspace = await Workspace.find("./fixtures/monorepo");
    const packages: Package[] = await workspace.findPackages();
    assert.notEqual(packages.length, 0);
  });

  it("returns a package graph", async () => {
    const dir = path.resolve(__dirname, "./fixtures/monorepo");
    const workspace = await Workspace.find(dir);
    const packages = await workspace.findPackagesWithGraph();

    assert.equal(Object.keys(packages).length, 2);

    const pkg1 = packages["apps/app"];
    const pkg2 = packages["packages/ui"];

    assert.deepEqual(pkg1.dependencies, ["packages/ui"]);
    assert.deepEqual(pkg1.dependents, []);

    assert.deepEqual(pkg2.dependencies, []);
    assert.deepEqual(pkg2.dependents, ["apps/app"]);
  });
});
