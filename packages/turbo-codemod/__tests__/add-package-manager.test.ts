import fs from "fs-extra";
import * as turboWorkspaces from "@turbo/workspaces";
import * as turboUtils from "@turbo/utils";
import { setupTestFixtures } from "@turbo/test-utils";
import { transformer } from "../src/transforms/add-package-manager";
import type { TransformerResults } from "../src/runner";
import type { TransformerOptions } from "../src/types";
import { getWorkspaceDetailsMockReturnValue } from "./test-utils";

jest.mock("@turbo/workspaces", () => ({
  __esModule: true,
  ...jest.requireActual("@turbo/workspaces"),
}));

interface TestCase {
  name: string;
  fixture: string;
  existingPackageManagerString: string | undefined;
  packageManager: turboUtils.PackageManager;
  packageManagerVersion: string;
  options: TransformerOptions;
  result: TransformerResults;
}

const TEST_CASES: Array<TestCase> = [
  {
    name: "basic",
    fixture: "no-package-manager",
    existingPackageManagerString: undefined,
    packageManager: "npm",
    packageManagerVersion: "7.0.0",
    options: { force: false, dryRun: false, print: false },
    result: {
      changes: {
        "package.json": {
          action: "modified",
          additions: 1,
          deletions: 0,
        },
      },
    },
  },
  {
    name: "dry",
    fixture: "no-package-manager",
    existingPackageManagerString: undefined,
    packageManager: "npm",
    packageManagerVersion: "7.0.0",
    options: { force: false, dryRun: true, print: false },
    result: {
      changes: {
        "package.json": {
          action: "skipped",
          additions: 1,
          deletions: 0,
        },
      },
    },
  },
  {
    name: "print",
    fixture: "no-package-manager",
    existingPackageManagerString: undefined,
    packageManager: "yarn",
    packageManagerVersion: "1.2.3",
    options: { force: false, dryRun: false, print: true },
    result: {
      changes: {
        "package.json": {
          action: "modified",
          additions: 1,
          deletions: 0,
        },
      },
    },
  },
  {
    name: "print & dry",
    fixture: "no-package-manager",
    existingPackageManagerString: undefined,
    packageManager: "pnpm",
    packageManagerVersion: "1.2.3",
    options: { force: false, dryRun: true, print: true },
    result: {
      changes: {
        "package.json": {
          action: "skipped",
          additions: 1,
          deletions: 0,
        },
      },
    },
  },
  {
    name: "basic",
    fixture: "has-package-manager",
    existingPackageManagerString: "npm@1.2.3",
    packageManager: "npm",
    packageManagerVersion: "1.2.3",
    options: { force: false, dryRun: false, print: false },
    result: {
      changes: {},
    },
  },
  {
    name: "basic",
    fixture: "wrong-package-manager",
    existingPackageManagerString: "turbo@1.7.0",
    packageManager: "pnpm",
    packageManagerVersion: "1.2.3",
    options: { force: false, dryRun: false, print: false },
    result: {
      changes: {},
    },
  },
];

describe("add-package-manager-2", () => {
  const { useFixture } = setupTestFixtures({
    directory: __dirname,
    test: "add-package-manager",
  });

  test.each(TEST_CASES)(
    "$fixture - $name with $packageManager@$packageManagerVersion using $options",
    async ({
      fixture,
      existingPackageManagerString,
      packageManager,
      packageManagerVersion,
      options,
      result,
    }) => {
      // load the fixture for the test
      const { root, read } = useFixture({ fixture });

      // mock out workspace and version detection so we're not dependent on our actual repo
      const mockGetAvailablePackageManagers = jest
        .spyOn(turboUtils, "getAvailablePackageManagers")
        .mockResolvedValue({
          pnpm: packageManager === "pnpm" ? packageManagerVersion : undefined,
          npm: packageManager === "npm" ? packageManagerVersion : undefined,
          yarn: packageManager === "yarn" ? packageManagerVersion : undefined,
          bun: packageManager === "bun" ? packageManagerVersion : undefined,
        });

      const mockGetWorkspaceDetails = jest
        .spyOn(turboWorkspaces, "getWorkspaceDetails")
        .mockResolvedValue(
          getWorkspaceDetailsMockReturnValue({
            root,
            packageManager,
          })
        );

      // verify package manager
      expect(JSON.parse(read("package.json") || "{}").packageManager).toEqual(
        existingPackageManagerString
      );

      // run the transformer
      const transformerResult = await transformer({
        root,
        options,
      });

      if (existingPackageManagerString === undefined) {
        expect(mockGetAvailablePackageManagers).toHaveBeenCalled();
        expect(mockGetWorkspaceDetails).toHaveBeenCalled();
      }

      expect(JSON.parse(read("package.json") || "{}").packageManager).toEqual(
        options.dryRun
          ? undefined
          : existingPackageManagerString ||
              `${packageManager}@${packageManagerVersion}`
      );

      // result should be correct
      expect(transformerResult.changes).toMatchObject(result.changes);

      // run the transformer again to ensure nothing changes on a second run
      const repeatResult = await transformer({
        root,
        options,
      });
      expect(repeatResult.fatalError).toBeUndefined();
      expect(repeatResult.changes).toMatchObject({});

      mockGetAvailablePackageManagers.mockRestore();
      mockGetWorkspaceDetails.mockRestore();
    }
  );

  describe("errors", () => {
    test("unable to determine workspace manager", async () => {
      // load the fixture for the test
      const { root, read } = useFixture({ fixture: "no-package-manager" });

      const mockGetWorkspaceDetails = jest
        .spyOn(turboWorkspaces, "getWorkspaceDetails")
        .mockRejectedValue(undefined);

      // package manager should not exist
      expect(
        JSON.parse(read("package.json") || "{}").packageManager
      ).toBeUndefined();
      // run the transformer
      const result = await transformer({
        root,
        options: { force: false, dryRun: false, print: false },
      });

      expect(mockGetWorkspaceDetails).toHaveBeenCalledTimes(1);

      // result should be correct
      expect(result.fatalError?.message).toMatch(
        /Unable to determine package manager for .*?/
      );

      mockGetWorkspaceDetails.mockRestore();
    });

    test("unable to determine package manager version", async () => {
      // load the fixture for the test
      const { root, read } = useFixture({ fixture: "no-package-manager" });

      const mockGetAvailablePackageManagers = jest
        .spyOn(turboUtils, "getAvailablePackageManagers")
        .mockResolvedValue({
          pnpm: undefined,
          npm: undefined,
          yarn: undefined,
          bun: undefined,
        });

      const mockGetWorkspaceDetails = jest
        .spyOn(turboWorkspaces, "getWorkspaceDetails")
        .mockResolvedValue(
          getWorkspaceDetailsMockReturnValue({
            root,
            packageManager: "npm",
          })
        );

      // package manager should not exist
      expect(
        JSON.parse(read("package.json") || "{}").packageManager
      ).toBeUndefined();
      // run the transformer
      const result = await transformer({
        root,
        options: { force: false, dryRun: false, print: false },
      });

      expect(mockGetAvailablePackageManagers).toHaveBeenCalledTimes(1);
      expect(mockGetWorkspaceDetails).toHaveBeenCalledTimes(1);

      // result should be correct
      expect(result.fatalError?.message).toMatch(
        /Unable to determine package manager version for .*?/
      );

      mockGetAvailablePackageManagers.mockRestore();
      mockGetWorkspaceDetails.mockRestore();
    });

    test("unable to write json", async () => {
      // load the fixture for the test
      const { root, read } = useFixture({ fixture: "no-package-manager" });

      const packageManager = "pnpm";
      const packageManagerVersion = "1.2.3";

      // mock out workspace and version detection so we're not dependent on our actual repo
      const mockGetAvailablePackageManagers = jest
        .spyOn(turboUtils, "getAvailablePackageManagers")
        .mockResolvedValue({
          pnpm: packageManagerVersion,
          npm: undefined,
          yarn: undefined,
          bun: undefined,
        });

      const mockGetWorkspaceDetails = jest
        .spyOn(turboWorkspaces, "getWorkspaceDetails")
        .mockResolvedValue(
          getWorkspaceDetailsMockReturnValue({
            root,
            packageManager,
          })
        );

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
      const result = await transformer({
        root,
        options: { force: false, dryRun: false, print: false },
      });

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
      mockGetAvailablePackageManagers.mockRestore();
      mockGetWorkspaceDetails.mockRestore();
    });
  });
});
