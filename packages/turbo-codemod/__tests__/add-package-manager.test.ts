import { transformer } from "../src/transforms/add-package-manager";
import { setupTestFixtures } from "@turbo/test-utils";
import fs from "fs-extra";
import * as getPackageManager from "../src/utils/getPackageManager";
import * as getPackageManagerVersion from "../src/utils/getPackageManagerVersion";

describe("add-package-manager", () => {
  const { useFixture } = setupTestFixtures({
    directory: __dirname,
    test: "add-package-manager",
  });
  test("no package manager - basic", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "no-package-manager" });

    const packageManager = "pnpm";
    const packageManagerVersion = "1.2.3";

    // mock out workspace and version detection so we're not dependent on our actual repo
    const mockGetPackageManagerVersion = jest
      .spyOn(getPackageManagerVersion, "default")
      .mockReturnValue(packageManagerVersion);

    const mockGetPackageManager = jest
      .spyOn(getPackageManager, "default")
      .mockReturnValue(packageManager);

    // package manager should not exist
    expect(
      JSON.parse(read("package.json") || "{}").packageManager
    ).toBeUndefined();
    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    expect(mockGetPackageManager).toHaveBeenCalledWith({ directory: root });
    expect(mockGetPackageManagerVersion).toHaveBeenCalledWith(
      packageManager,
      root
    );

    // package manager should now exist
    expect(JSON.parse(read("package.json") || "{}").packageManager).toBe(
      `${packageManager}@${packageManagerVersion}`
    );
    // result should be correct
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "package.json": Object {
          "action": "modified",
          "additions": 1,
          "deletions": 0,
        },
      }
    `);

    mockGetPackageManagerVersion.mockRestore();
    mockGetPackageManager.mockRestore();
  });

  test("no package manager - repeat run", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "no-package-manager" });

    const packageManager = "pnpm";
    const packageManagerVersion = "1.2.3";

    // mock out workspace and version detection so we're not dependent on our actual repo
    const mockGetPackageManagerVersion = jest
      .spyOn(getPackageManagerVersion, "default")
      .mockReturnValue(packageManagerVersion);

    const mockGetPackageManager = jest
      .spyOn(getPackageManager, "default")
      .mockReturnValue(packageManager);

    // package manager should not exist
    expect(
      JSON.parse(read("package.json") || "{}").packageManager
    ).toBeUndefined();
    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    expect(mockGetPackageManager).toHaveBeenCalledWith({ directory: root });
    expect(mockGetPackageManagerVersion).toHaveBeenCalled();
    expect(mockGetPackageManagerVersion).toHaveBeenCalledWith(
      packageManager,
      root
    );

    // package manager should now exist
    expect(JSON.parse(read("package.json") || "{}").packageManager).toBe(
      `${packageManager}@${packageManagerVersion}`
    );
    // result should be correct
    expect(result.fatalError).toBeUndefined();
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "package.json": Object {
          "action": "modified",
          "additions": 1,
          "deletions": 0,
        },
      }
    `);

    // run the transformer again to ensure nothing changes on a second run
    const repeatResult = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });
    expect(repeatResult.fatalError).toBeUndefined();
    expect(repeatResult.changes).toMatchInlineSnapshot(`
    Object {
      "package.json": Object {
        "action": "unchanged",
        "additions": 0,
        "deletions": 0,
      },
    }
  `);

    mockGetPackageManagerVersion.mockRestore();
    mockGetPackageManager.mockRestore();
  });

  test("no package manager - dry", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "no-package-manager" });

    const packageManager = "npm";
    const packageManagerVersion = "1.2.3";

    // mock out workspace and version detection so we're not dependent on our actual repo
    const mockGetPackageManagerVersion = jest
      .spyOn(getPackageManagerVersion, "default")
      .mockReturnValue(packageManagerVersion);
    const mockGetPackageManager = jest
      .spyOn(getPackageManager, "default")
      .mockReturnValue(packageManager);

    // package manager should not exist
    expect(
      JSON.parse(read("package.json") || "{}").packageManager
    ).toBeUndefined();
    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: true, print: false },
    });

    expect(mockGetPackageManager).toHaveBeenCalledWith({ directory: root });
    expect(mockGetPackageManagerVersion).toHaveBeenCalledWith(
      packageManager,
      root
    );

    // package manager should not exist
    expect(
      JSON.parse(read("package.json") || "{}").packageManager
    ).toBeUndefined();
    // result should be correct
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "package.json": Object {
          "action": "skipped",
          "additions": 1,
          "deletions": 0,
        },
      }
    `);

    mockGetPackageManagerVersion.mockRestore();
  });

  test("no package manager - print", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "no-package-manager" });

    const packageManager = "yarn";
    const packageManagerVersion = "1.2.3";

    // mock out workspace and version detection so we're not dependent on our actual repo
    const mockGetPackageManagerVersion = jest
      .spyOn(getPackageManagerVersion, "default")
      .mockReturnValue(packageManagerVersion);

    const mockGetPackageManager = jest
      .spyOn(getPackageManager, "default")
      .mockReturnValue(packageManager);

    // package manager should not exist
    expect(
      JSON.parse(read("package.json") || "{}").packageManager
    ).toBeUndefined();
    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: true },
    });

    expect(mockGetPackageManager).toHaveBeenCalledWith({ directory: root });
    expect(mockGetPackageManagerVersion).toHaveBeenCalledWith(
      packageManager,
      root
    );
    // package manager should now exist
    expect(JSON.parse(read("package.json") || "{}").packageManager).toBe(
      `${packageManager}@${packageManagerVersion}`
    );
    // result should be correct
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "package.json": Object {
          "action": "modified",
          "additions": 1,
          "deletions": 0,
        },
      }
    `);

    mockGetPackageManagerVersion.mockRestore();
  });

  test("no package manager - dry & print", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "no-package-manager" });

    const packageManager = "npm";
    const packageManagerVersion = "1.2.3";

    // mock out workspace and version detection so we're not dependent on our actual repo
    const mockGetPackageManagerVersion = jest
      .spyOn(getPackageManagerVersion, "default")
      .mockReturnValue(packageManagerVersion);

    const mockGetPackageManager = jest
      .spyOn(getPackageManager, "default")
      .mockReturnValue(packageManager);

    // package manager should not exist
    expect(
      JSON.parse(read("package.json") || "{}").packageManager
    ).toBeUndefined();
    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: true, print: true },
    });

    expect(mockGetPackageManager).toHaveBeenCalledWith({ directory: root });
    expect(mockGetPackageManagerVersion).toHaveBeenCalledWith(
      packageManager,
      root
    );

    // package manager should not exist
    expect(
      JSON.parse(read("package.json") || "{}").packageManager
    ).toBeUndefined();
    // result should be correct
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "package.json": Object {
          "action": "skipped",
          "additions": 1,
          "deletions": 0,
        },
      }
    `);

    mockGetPackageManagerVersion.mockRestore();
    mockGetPackageManager.mockRestore();
  });

  test("package manager already exists", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "has-package-manager" });
    const packageManager = "npm";
    const packageManagerVersion = "1.2.3";

    // mock out workspace and version detection so we're not dependent on our actual repo
    const mockGetPackageManagerVersion = jest
      .spyOn(getPackageManagerVersion, "default")
      .mockReturnValue(packageManagerVersion);

    const mockGetPackageManager = jest
      .spyOn(getPackageManager, "default")
      .mockReturnValue(packageManager);

    // package manager should exist
    expect(JSON.parse(read("package.json") || "{}").packageManager).toBe(
      `${packageManager}@${packageManagerVersion}`
    );
    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    expect(mockGetPackageManager).toHaveBeenCalledWith({ directory: root });
    expect(mockGetPackageManagerVersion).toHaveBeenCalledWith(
      packageManager,
      root
    );

    // package manager should still exist
    expect(JSON.parse(read("package.json") || "{}").packageManager).toBe(
      `${packageManager}@${packageManagerVersion}`
    );
    // result should be correct
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "package.json": Object {
          "action": "unchanged",
          "additions": 0,
          "deletions": 0,
        },
      }
    `);

    mockGetPackageManagerVersion.mockRestore();
    mockGetPackageManager.mockRestore();
  });

  test("package manager exists but is wrong", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "wrong-package-manager" });

    const packageManager = "pnpm";
    const packageManagerVersion = "1.2.3";

    // mock out workspace and version detection so we're not dependent on our actual repo
    const mockGetPackageManagerVersion = jest
      .spyOn(getPackageManagerVersion, "default")
      .mockReturnValue(packageManagerVersion);

    const mockGetPackageManager = jest
      .spyOn(getPackageManager, "default")
      .mockReturnValue(packageManager);

    // package manager should exist
    expect(JSON.parse(read("package.json") || "{}").packageManager).toBe(
      "turbo@1.7.0"
    );
    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    expect(mockGetPackageManager).toHaveBeenCalledWith({ directory: root });
    expect(mockGetPackageManagerVersion).toHaveBeenCalledWith(
      packageManager,
      root
    );

    // package manager should still exist
    expect(JSON.parse(read("package.json") || "{}").packageManager).toBe(
      `${packageManager}@${packageManagerVersion}`
    );
    // result should be correct
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "package.json": Object {
          "action": "modified",
          "additions": 1,
          "deletions": 1,
        },
      }
    `);

    mockGetPackageManagerVersion.mockRestore();
    mockGetPackageManager.mockRestore();
  });

  test("errors when unable to determine workspace manager", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "no-package-manager" });

    const mockGetPackageManager = jest
      .spyOn(getPackageManager, "default")
      .mockReturnValue(undefined);

    // package manager should not exist
    expect(
      JSON.parse(read("package.json") || "{}").packageManager
    ).toBeUndefined();
    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    expect(mockGetPackageManager).toHaveBeenCalledTimes(1);
    expect(mockGetPackageManager).toHaveBeenCalledWith({ directory: root });

    // result should be correct
    // result should be correct
    expect(result.fatalError?.message).toMatch(
      /Unable to determine package manager for .*?/
    );

    mockGetPackageManager.mockRestore();
  });

  test("errors when unable to determine package manager", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "no-package-manager" });

    const mockGetPackageManagerVersion = jest
      .spyOn(getPackageManagerVersion, "default")
      .mockImplementation(() => {
        throw new Error("package manager not supported");
      });

    // package manager should not exist
    expect(
      JSON.parse(read("package.json") || "{}").packageManager
    ).toBeUndefined();
    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    expect(mockGetPackageManagerVersion).toHaveBeenCalledTimes(1);

    // result should be correct
    expect(result.fatalError?.message).toMatch(
      /Unable to determine package manager version for .*?/
    );

    mockGetPackageManagerVersion.mockRestore();
  });

  test("errors when unable to write json", () => {
    // load the fixture for the test
    const { root, read } = useFixture({ fixture: "no-package-manager" });

    const packageManager = "pnpm";
    const packageManagerVersion = "1.2.3";

    // mock out workspace and version detection so we're not dependent on our actual repo
    const mockGetPackageManagerVersion = jest
      .spyOn(getPackageManagerVersion, "default")
      .mockReturnValue(packageManagerVersion);

    const mockGetPackageManager = jest
      .spyOn(getPackageManager, "default")
      .mockReturnValue(packageManager);

    const mockWriteJsonSync = jest
      .spyOn(fs, "writeJsonSync")
      .mockImplementation(() => {
        throw new Error("could not write file");
      });

    // package manager should not exist
    expect(
      JSON.parse(read("package.json") || "{}").packageManager
    ).toBeUndefined();
    // run the transformer
    const result = transformer({
      root,
      options: { force: false, dry: false, print: false },
    });

    expect(mockGetPackageManager).toHaveBeenCalledWith({ directory: root });
    expect(mockGetPackageManagerVersion).toHaveBeenCalledWith(
      packageManager,
      root
    );

    // package manager should still not exist (we couldn't write it)
    expect(
      JSON.parse(read("package.json") || "{}").packageManager
    ).toBeUndefined();

    // result should be correct
    expect(result.fatalError?.message).toMatch(
      "Encountered an error while transforming files"
    );
    expect(result.changes).toMatchInlineSnapshot(`
      Object {
        "package.json": Object {
          "action": "error",
          "additions": 1,
          "deletions": 0,
          "error": [Error: could not write file],
        },
      }
    `);

    mockWriteJsonSync.mockRestore();
    mockGetPackageManagerVersion.mockRestore();
    mockGetPackageManager.mockRestore();
  });
});
