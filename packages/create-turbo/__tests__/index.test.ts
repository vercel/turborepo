import path from "node:path";
import childProcess from "node:child_process";
import picocolors from "picocolors";
import { setupTestFixtures, spyConsole, spyExit } from "@turbo/test-utils";
import { logger } from "@turbo/utils";
import type { PackageManager } from "@turbo/utils";
// imports for mocks
import * as turboWorkspaces from "@turbo/workspaces";
import { CreateTurboTelemetry, TelemetryConfig } from "@turbo/telemetry";
import * as turboUtils from "@turbo/utils";
import { describe, it, expect, jest } from "@jest/globals";
import type { CreateCommandArgument } from "../src/commands/create/types";
import * as gitUtils from "../src/utils/git";
import { create } from "../src/commands/create";
import { getWorkspaceDetailsMockReturnValue } from "./test-utils";

jest.mock<typeof import("@turbo/workspaces")>("@turbo/workspaces", () => ({
  __esModule: true,
  ...jest.requireActual("@turbo/workspaces"),
}));

describe("create-turbo", () => {
  const { useFixture } = setupTestFixtures({
    directory: path.join(__dirname, "../"),
    options: { emptyFixture: true },
  });

  const mockConsole = spyConsole();
  const mockExit = spyExit();
  const telemetry = new CreateTurboTelemetry({
    api: "https://example.com",
    packageInfo: {
      name: "create-turbo",
      version: "1.0.0",
    },
    config: new TelemetryConfig({
      configPath: "test-config-path",
      config: {
        telemetry_enabled: false,
        telemetry_id: "telemetry-test-id",
        telemetry_salt: "telemetry-salt",
      },
    }),
  });

  it.each<{ packageManager: PackageManager }>([
    { packageManager: "yarn" },
    { packageManager: "npm" },
    { packageManager: "pnpm" },
    { packageManager: "bun" },
  ])(
    "outputs expected console messages when using $packageManager (option)",
    async ({ packageManager }) => {
      const { root } = useFixture({ fixture: "create-turbo" });

      const availableScripts = ["build", "test", "dev"];

      const mockAvailablePackageManagers = jest
        .spyOn(turboUtils, "getAvailablePackageManagers")
        .mockResolvedValue({
          npm: "8.19.2",
          yarn: "1.22.10",
          pnpm: "7.22.2",
          bun: "1.0.1",
        });

      const mockCreateProject = jest
        .spyOn(turboUtils, "createProject")
        .mockResolvedValue({
          cdPath: "",
          hasPackageJson: true,
          availableScripts,
        });

      const mockGetWorkspaceDetails = jest
        .spyOn(turboWorkspaces, "getWorkspaceDetails")
        .mockResolvedValue(
          getWorkspaceDetailsMockReturnValue({
            root,
            packageManager,
          })
        );

      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockImplementation(() => {
          return "success";
        });

      await create(root as CreateCommandArgument, {
        packageManager,
        skipInstall: true,
        example: "default",
        telemetry,
      });

      const expected = `${picocolors.bold(
        logger.turboGradient(">>> Success!")
      )} Created your Turborepo at ${picocolors.green(
        path.relative(process.cwd(), root)
      )}`;
      expect(mockConsole.log).toHaveBeenCalledWith(expected);
      expect(mockConsole.log).toHaveBeenCalledWith();
      expect(mockConsole.log).toHaveBeenCalledWith(
        picocolors.bold("To get started:")
      );

      expect(mockConsole.log).toHaveBeenCalledWith(
        picocolors.cyan("Library packages")
      );

      expect(mockConsole.log).toHaveBeenCalledWith(
        "- Run commands with Turborepo:"
      );

      availableScripts.forEach((script) => {
        expect(mockConsole.log).toHaveBeenCalledWith(
          expect.stringContaining(
            picocolors.cyan(`${packageManager} run ${script}`)
          )
        );
      });

      expect(mockConsole.log).toHaveBeenCalledWith(
        "- Run a command twice to hit cache"
      );

      mockAvailablePackageManagers.mockRestore();
      mockCreateProject.mockRestore();
      mockGetWorkspaceDetails.mockRestore();
      mockExecSync.mockRestore();
    }
  );

  it.each<{ packageManager: PackageManager }>([
    { packageManager: "yarn" },
    { packageManager: "npm" },
    { packageManager: "pnpm" },
    { packageManager: "bun" },
  ])(
    "outputs expected console messages when using $packageManager (arg)",
    async ({ packageManager }) => {
      const { root } = useFixture({ fixture: "create-turbo" });

      const availableScripts = ["build", "test", "dev"];

      const mockAvailablePackageManagers = jest
        .spyOn(turboUtils, "getAvailablePackageManagers")
        .mockResolvedValue({
          npm: "8.19.2",
          yarn: "1.22.10",
          pnpm: "7.22.2",
          bun: "1.0.1",
        });

      const mockCreateProject = jest
        .spyOn(turboUtils, "createProject")
        .mockResolvedValue({
          cdPath: "",
          hasPackageJson: true,
          availableScripts,
        });

      const mockGetWorkspaceDetails = jest
        .spyOn(turboWorkspaces, "getWorkspaceDetails")
        .mockResolvedValue(
          getWorkspaceDetailsMockReturnValue({
            root,
            packageManager,
          })
        );

      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockImplementation(() => {
          return "success";
        });

      await create(root as CreateCommandArgument, {
        packageManager,
        skipInstall: true,
        example: "default",
        telemetry,
      });

      const expected = `${picocolors.bold(
        logger.turboGradient(">>> Success!")
      )} Created your Turborepo at ${picocolors.green(
        path.relative(process.cwd(), root)
      )}`;
      expect(mockConsole.log).toHaveBeenCalledWith(expected);
      expect(mockConsole.log).toHaveBeenCalledWith();
      expect(mockConsole.log).toHaveBeenCalledWith(
        picocolors.bold("To get started:")
      );

      expect(mockConsole.log).toHaveBeenCalledWith(
        picocolors.cyan("Library packages")
      );

      expect(mockConsole.log).toHaveBeenCalledWith(
        "- Run commands with Turborepo:"
      );

      availableScripts.forEach((script) => {
        expect(mockConsole.log).toHaveBeenCalledWith(
          expect.stringContaining(
            picocolors.cyan(`${packageManager} run ${script}`)
          )
        );
      });

      expect(mockConsole.log).toHaveBeenCalledWith(
        "- Run a command twice to hit cache"
      );
      mockAvailablePackageManagers.mockRestore();
      mockCreateProject.mockRestore();
      mockGetWorkspaceDetails.mockRestore();
      mockExecSync.mockRestore();
    }
  );

  it("throws correct error message when a download error is encountered", async () => {
    const { root } = useFixture({ fixture: "create-turbo" });
    const packageManager = "pnpm";
    const mockAvailablePackageManagers = jest
      .spyOn(turboUtils, "getAvailablePackageManagers")
      .mockResolvedValue({
        npm: "8.19.2",
        yarn: "1.22.10",
        pnpm: "7.22.2",
        bun: "1.0.1",
      });

    const mockCreateProject = jest
      .spyOn(turboUtils, "createProject")
      .mockRejectedValue(new turboUtils.DownloadError("Could not connect"));

    const mockGetWorkspaceDetails = jest
      .spyOn(turboWorkspaces, "getWorkspaceDetails")
      .mockResolvedValue(
        getWorkspaceDetailsMockReturnValue({
          root,
          packageManager,
        })
      );

    const mockExecSync = jest
      .spyOn(childProcess, "execSync")
      .mockImplementation(() => {
        return "success";
      });

    await create(root as CreateCommandArgument, {
      packageManager,
      skipInstall: true,
      example: "default",
      telemetry,
    });

    expect(mockConsole.error).toHaveBeenCalledTimes(2);
    expect(mockConsole.error).toHaveBeenNthCalledWith(
      1,
      logger.turboRed(picocolors.bold(">>>")),
      picocolors.red("Unable to download template from GitHub")
    );
    expect(mockConsole.error).toHaveBeenNthCalledWith(
      2,
      logger.turboRed(picocolors.bold(">>>")),
      picocolors.red("Could not connect")
    );
    expect(mockExit.exit).toHaveBeenCalledWith(1);

    mockAvailablePackageManagers.mockRestore();
    mockCreateProject.mockRestore();
    mockGetWorkspaceDetails.mockRestore();
    mockExecSync.mockRestore();
  });

  it("removes .git directory when --no-git flag is used", async () => {
    const { root } = useFixture({ fixture: "create-turbo-no-git" });
    const packageManager = "npm";

    const mockAvailablePackageManagers = jest
      .spyOn(turboUtils, "getAvailablePackageManagers")
      .mockResolvedValue({
        npm: "8.19.2",
        yarn: "1.22.10",
        pnpm: "7.22.2",
        bun: "1.0.1",
      });

    const mockCreateProject = jest
      .spyOn(turboUtils, "createProject")
      .mockResolvedValue({
        cdPath: "",
        hasPackageJson: true,
        availableScripts: ["build", "test", "dev"],
      });

    const mockGetWorkspaceDetails = jest
      .spyOn(turboWorkspaces, "getWorkspaceDetails")
      .mockResolvedValue(
        getWorkspaceDetailsMockReturnValue({
          root,
          packageManager,
        })
      );

    const mockExecSync = jest
      .spyOn(childProcess, "execSync")
      .mockImplementation(() => {
        return "success";
      });

    const mockRemoveGitDirectory = jest
      .spyOn(gitUtils, "removeGitDirectory")
      .mockReturnValue(true);

    await create(root as CreateCommandArgument, {
      packageManager,
      skipInstall: true,
      example: "default",
      noGit: true,
      telemetry,
    });

    expect(mockRemoveGitDirectory).toHaveBeenCalledWith(root);

    mockAvailablePackageManagers.mockRestore();
    mockCreateProject.mockRestore();
    mockGetWorkspaceDetails.mockRestore();
    mockExecSync.mockRestore();
    mockRemoveGitDirectory.mockRestore();
  });

  it("does not remove .git directory when --no-git flag is not used", async () => {
    const { root } = useFixture({ fixture: "create-turbo-with-git" });
    const packageManager = "npm";

    const mockAvailablePackageManagers = jest
      .spyOn(turboUtils, "getAvailablePackageManagers")
      .mockResolvedValue({
        npm: "8.19.2",
        yarn: "1.22.10",
        pnpm: "7.22.2",
        bun: "1.0.1",
      });

    const mockCreateProject = jest
      .spyOn(turboUtils, "createProject")
      .mockResolvedValue({
        cdPath: "",
        hasPackageJson: true,
        availableScripts: ["build", "test", "dev"],
      });

    const mockGetWorkspaceDetails = jest
      .spyOn(turboWorkspaces, "getWorkspaceDetails")
      .mockResolvedValue(
        getWorkspaceDetailsMockReturnValue({
          root,
          packageManager,
        })
      );

    const mockExecSync = jest
      .spyOn(childProcess, "execSync")
      .mockImplementation(() => {
        return "success";
      });

    const mockRemoveGitDirectory = jest
      .spyOn(gitUtils, "removeGitDirectory")
      .mockReturnValue(true);

    await create(root as CreateCommandArgument, {
      packageManager,
      skipInstall: true,
      example: "default",
      noGit: false,
      telemetry,
    });

    expect(mockRemoveGitDirectory).not.toHaveBeenCalled();

    mockAvailablePackageManagers.mockRestore();
    mockCreateProject.mockRestore();
    mockGetWorkspaceDetails.mockRestore();
    mockExecSync.mockRestore();
    mockRemoveGitDirectory.mockRestore();
  });
});
