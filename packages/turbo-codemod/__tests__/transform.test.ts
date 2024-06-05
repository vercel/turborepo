import * as turboWorkspaces from "@turbo/workspaces";
import * as turboUtils from "@turbo/utils";
import { setupTestFixtures, spyExit } from "@turbo/test-utils";
import { transform } from "../src/commands/transform";
import * as checkGitStatus from "../src/utils/checkGitStatus";
import type { MigrateCommandArgument } from "../src/commands";
import { getWorkspaceDetailsMockReturnValue } from "./test-utils";

jest.mock("@turbo/workspaces", () => ({
  __esModule: true,
  ...jest.requireActual("@turbo/workspaces"),
}));

describe("transform", () => {
  const mockExit = spyExit();
  const { useFixture } = setupTestFixtures({
    directory: __dirname,
    test: "transform",
  });

  it("runs the selected transform", async () => {
    const { root, readJson } = useFixture({
      fixture: "basic",
    });

    const packageManager = "pnpm";
    const packageManagerVersion = "1.2.3";

    // setup mocks
    const mockedCheckGitStatus = jest
      .spyOn(checkGitStatus, "checkGitStatus")
      .mockReturnValue(undefined);
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

    await transform("add-package-manager", root as MigrateCommandArgument, {
      list: false,
      force: false,
      dryRun: false,
      print: false,
    });

    expect(readJson("package.json")).toStrictEqual({
      dependencies: {},
      devDependencies: {
        turbo: "1.0.0",
      },
      name: "transform-basic",
      packageManager: "pnpm@1.2.3",
      version: "1.0.0",
    });

    // verify mocks were called
    expect(mockedCheckGitStatus).toHaveBeenCalled();
    expect(mockGetAvailablePackageManagers).toHaveBeenCalled();
    expect(mockGetWorkspaceDetails).toHaveBeenCalled();

    // restore mocks
    mockedCheckGitStatus.mockRestore();
    mockGetAvailablePackageManagers.mockRestore();
    mockGetWorkspaceDetails.mockRestore();
  });

  it("runs the selected transform - dry & print", async () => {
    const { root, readJson } = useFixture({
      fixture: "basic",
    });

    const packageManager = "pnpm";
    const packageManagerVersion = "1.2.3";

    // setup mocks
    const mockedCheckGitStatus = jest
      .spyOn(checkGitStatus, "checkGitStatus")
      .mockReturnValue(undefined);
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

    await transform("add-package-manager", root, {
      list: false,
      force: false,
      dryRun: true,
      print: true,
    });

    expect(readJson("package.json")).toStrictEqual({
      dependencies: {},
      devDependencies: {
        turbo: "1.0.0",
      },
      name: "transform-basic",
      version: "1.0.0",
    });

    // verify mocks were called
    expect(mockedCheckGitStatus).not.toHaveBeenCalled();
    expect(mockGetAvailablePackageManagers).toHaveBeenCalled();
    expect(mockGetWorkspaceDetails).toHaveBeenCalled();

    // restore mocks
    mockedCheckGitStatus.mockRestore();
    mockGetAvailablePackageManagers.mockRestore();
    mockGetWorkspaceDetails.mockRestore();
  });

  it("lists transforms", async () => {
    const { root } = useFixture({
      fixture: "basic",
    });

    await transform("add-package-manager", root, {
      list: true,
      force: false,
      dryRun: false,
      print: false,
    });

    expect(mockExit.exit).toHaveBeenCalledWith(0);
  });

  it("exits on invalid transform", async () => {
    const { root } = useFixture({
      fixture: "basic",
    });

    await transform("not-a-real-option", root, {
      list: false,
      force: false,
      dryRun: false,
      print: false,
    });

    expect(mockExit.exit).toHaveBeenCalledWith(1);
  });

  it("exits on invalid directory", async () => {
    useFixture({
      fixture: "basic",
    });

    await transform("add-package-manager", "~/path/that/does/not/exist", {
      list: false,
      force: false,
      dryRun: false,
      print: false,
    });

    expect(mockExit.exit).toHaveBeenCalledWith(1);
  });
});
