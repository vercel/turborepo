import { describe, it } from "node:test";
import { strict as assert } from "node:assert";
import * as path from "node:path";
import { Workspace, Package } from "../js/dist/index.js";

const MONOREPO_PATH = path.resolve(__dirname, "./fixtures/monorepo");

describe("findPackages", () => {
  it("enumerates packages", async () => {
    const workspace = await Workspace.find(MONOREPO_PATH);
    const packages: Package[] = await workspace.findPackages();
    assert.notEqual(packages.length, 0);
  });

  it("returns a package graph", async () => {
    const workspace = await Workspace.find(MONOREPO_PATH);
    const packages = await workspace.findPackagesWithGraph();

    assert.equal(Object.keys(packages).length, 2);

    const pkg1 = packages["apps/app"];
    const pkg2 = packages["packages/ui"];

    assert.deepEqual(pkg1.dependencies, ["packages/ui"]);
    assert.deepEqual(pkg1.dependents, []);

    assert.deepEqual(pkg2.dependencies, []);
    assert.deepEqual(pkg2.dependents, ["apps/app"]);
  });

  it("returns the package for a given path", async () => {
    const workspace = await Workspace.find(MONOREPO_PATH);

    for (const [filePath, result] of [
      ["apps/app/src/util/useful-file.ts", "app-a"],
      [
        "apps/app/src/very/deeply/nested/file/that/is/deep/as/can/be/with/a/package.ts",
        "app-a",
      ],
      ["apps/app/src/util/non-typescript-file.txt", "app-a"],
      ["apps/app/src/a-directory", "app-a"],
      ["apps/app/package.json", "app-a"],
      ["apps/app/tsconfig.json", "app-a"],
      ["apps/app", "app-a"], // The root of a package is still "within" a package!
      ["apps/app/", "app-a"], // Trailing-slash should be ignored
      ["packages/ui/pretty-stuff.css", "ui"],
      // This may be unintentional - I expected `findPackages` to return a nameless-package for `packages/blank` (whose
      // `package.json` is missing a `name` field), but instead there is no such package returned.
      ["packages/blank/nothing.null", undefined],
      ["packages/not-in-a-package", undefined],
      ["packages/not-in-a-package/but/very/deep/within/nothingness", undefined],
      ["", undefined],
      [".", undefined],
      ["..", undefined],
      ["apps/../apps/app/src", "app-a"],
      ["apps/app/src/util/../../../../apps/app", "app-a"],
      ["not a legal ^&(^) path", undefined],
      ["package.json", undefined],
      ["tsconfig.json", undefined],
    ]) {
      if (result === undefined) {
        assert.rejects(
          () => workspace.findPackageByPath(filePath!),
          `Expected rejection for ${filePath}`
        );
      } else {
        workspace
          .findPackageByPath(filePath!)
          .then((pkg) => {
            assert.equal(
              pkg.name,
              result,
              `Expected ${result} for ${filePath}`
            );
          })
          .catch((reason) => {
            assert.fail(`Expected success for ${filePath}, but got ${reason}`);
          });
      }
    }
  });
});
