import { getNewPkgName } from "../src/transforms/add-package-names";

describe("getNewPkgName", () => {
  it.each([
    {
      pkgPath: "/packages/ui/package.json",
      pkgName: "old-name",
      expected: "ui-old-name",
    },
    // scoped
    {
      pkgPath: "/packages/ui/package.json",
      pkgName: "@acme/name",
      expected: "@acme/ui-name",
    },
    // no name
    {
      pkgPath: "/packages/ui/package.json",
      pkgName: undefined,
      expected: "ui",
    },
  ])(
    "should return a new package name for pkgPath: $pkgPath and pkgName: $pkgName",
    ({ pkgPath, pkgName, expected }) => {
      const newName = getNewPkgName({ pkgPath, pkgName });
      expect(newName).toBe(expected);
    }
  );
});
