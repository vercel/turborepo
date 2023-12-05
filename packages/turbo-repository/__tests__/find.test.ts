import * as path from "node:path";
import {
  PackageManagerRoot,
  Package,
  PackageManager,
} from "../js/dist/index.js";

describe("find", () => {
  it("finds a package manager root", async () => {
    const packageManagerRoot = await PackageManagerRoot.find();
    console.log(packageManagerRoot);
    const expectedRoot = path.resolve(__dirname, "../../..");
    expect(packageManagerRoot.root).toBe(expectedRoot);
  });

  it("enumerates packages", async () => {
    const packageManagerRoot = await PackageManagerRoot.find();
    const packages: Package[] = await packageManagerRoot.packages();
    expect(packages.length).not.toBe(0);
  });

  it("finds a package manager", async () => {
    const packageManagerRoot = await PackageManagerRoot.find();
    const packageManager: PackageManager = packageManagerRoot.packageManager();
    expect(packageManager.name).toBe("pnpm");
  });
  // TODO: proper tests on real fixtures
});
