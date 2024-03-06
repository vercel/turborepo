import * as path from "node:path";
import { Workspace, Package } from "../js/dist/index.js";

describe("findPackages", () => {
  it("enumerates packages", async () => {
    const workspace = await Workspace.find("./fixtures/monorepo");
    const packages: Package[] = await workspace.findPackages();
    expect(packages.length).not.toBe(0);
  });

  it("returns a package graph", async () => {
    const dir = path.resolve(__dirname, "./fixtures/monorepo");
    const workspace = await Workspace.find(dir);
    const packages = await workspace.findPackagesWithGraph();

    expect(Object.keys(packages).length).toBe(2);

    const pkg1 = packages["apps/app"];
    const pkg2 = packages["packages/ui"];

    expect(pkg1.dependencies).toEqual(["packages/ui"]);
    expect(pkg1.dependents).toEqual([]);

    expect(pkg2.dependencies).toEqual([]);
    expect(pkg2.dependents).toEqual(["apps/app"]);
  });
});
