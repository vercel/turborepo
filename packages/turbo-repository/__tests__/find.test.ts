import * as path from "node:path";
import { PackageManagerRoot } from "../js/dist/index.js";

describe("find", () => {
  it("finds a package manager root", async () => {
    const packageManagerRoot = await PackageManagerRoot.find();
    console.log(packageManagerRoot);
    const expectedRoot = path.resolve(__dirname, "../../..");
    expect(packageManagerRoot.root).toBe(expectedRoot);
  });

  it("enumerates workspaces", async () => {
    const packageManagerRoot = await PackageManagerRoot.find();
    const workspaces = await packageManagerRoot.packages();
    expect(workspaces.length).not.toBe(0);
  });

  // TODO: proper tests on real fixtures
});
