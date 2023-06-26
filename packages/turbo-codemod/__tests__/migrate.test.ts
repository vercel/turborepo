import { MigrateCommandArgument } from "../src/commands";
import migrate from "../src/commands/migrate";
import { setupTestFixtures, spyExit } from "@turbo/test-utils";
import childProcess from "child_process";
import * as checkGitStatus from "../src/utils/checkGitStatus";
import * as getCurrentVersion from "../src/commands/migrate/steps/getCurrentVersion";
import * as getLatestVersion from "../src/commands/migrate/steps/getLatestVersion";
import * as getTurboUpgradeCommand from "../src/commands/migrate/steps/getTurboUpgradeCommand";
import * as workspaceImplementation from "../src/utils/getPackageManager";
import * as getPackageManagerVersion from "../src/utils/getPackageManagerVersion";

describe("migrate", () => {
  const mockExit = spyExit();
  const { useFixture } = setupTestFixtures({
    directory: __dirname,
    test: "migrate",
  });

  it("migrates from 1.0.0 to 1.7.0", async () => {
    const { root, readJson } = useFixture({
      fixture: "old-turbo",
    });

    const packageManager = "pnpm";
    const packageManagerVersion = "1.2.3";

    // setup mocks
    const mockedCheckGitStatus = jest
      .spyOn(checkGitStatus, "default")
      .mockReturnValue(undefined);
    const mockedGetCurrentVersion = jest
      .spyOn(getCurrentVersion, "default")
      .mockReturnValue("1.0.0");
    const mockedGetLatestVersion = jest
      .spyOn(getLatestVersion, "default")
      .mockResolvedValue("1.7.0");
    const mockedGetTurboUpgradeCommand = jest
      .spyOn(getTurboUpgradeCommand, "default")
      .mockReturnValue("pnpm install -g turbo@latest");
    const mockedGetPackageManagerVersion = jest
      .spyOn(getPackageManagerVersion, "default")
      .mockReturnValue(packageManagerVersion);
    const mockedGetWorkspaceImplementation = jest
      .spyOn(workspaceImplementation, "default")
      .mockReturnValue(packageManager);

    await migrate(root as MigrateCommandArgument, {
      force: false,
      dry: false,
      print: false,
      install: false,
    });

    expect(readJson("package.json")).toStrictEqual({
      dependencies: {},
      devDependencies: {
        turbo: "1.0.0",
      },
      name: "no-turbo-json",
      packageManager: "pnpm@1.2.3",
      version: "1.0.0",
    });
    expect(readJson("turbo.json")).toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      pipeline: {
        build: {
          outputs: [".next/**", "!.next/cache/**"],
        },
        dev: {
          cache: false,
        },
        lint: {},
        test: {
          outputs: ["dist/**", "build/**"],
        },
      },
    });

    // verify mocks were called
    expect(mockedCheckGitStatus).toHaveBeenCalled();
    expect(mockedGetCurrentVersion).toHaveBeenCalled();
    expect(mockedGetLatestVersion).toHaveBeenCalled();
    expect(mockedGetTurboUpgradeCommand).toHaveBeenCalled();
    expect(mockedGetPackageManagerVersion).toHaveBeenCalled();
    expect(mockedGetWorkspaceImplementation).toHaveBeenCalled();

    // restore mocks
    mockedCheckGitStatus.mockRestore();
    mockedGetCurrentVersion.mockRestore();
    mockedGetLatestVersion.mockRestore();
    mockedGetTurboUpgradeCommand.mockRestore();
    mockedGetPackageManagerVersion.mockRestore();
    mockedGetWorkspaceImplementation.mockRestore();
  });

  it("migrates from 1.0.0 to 1.2.0 (dry run)", async () => {
    const { root, readJson } = useFixture({
      fixture: "old-turbo",
    });

    const packageManager = "pnpm";
    const packageManagerVersion = "1.2.3";

    // setup mocks
    const mockedCheckGitStatus = jest
      .spyOn(checkGitStatus, "default")
      .mockReturnValue(undefined);
    const mockedGetCurrentVersion = jest
      .spyOn(getCurrentVersion, "default")
      .mockReturnValue("1.0.0");
    const mockedGetLatestVersion = jest
      .spyOn(getLatestVersion, "default")
      .mockResolvedValue("1.2.0");
    const mockedGetTurboUpgradeCommand = jest
      .spyOn(getTurboUpgradeCommand, "default")
      .mockReturnValue("pnpm install -g turbo@latest");
    const mockedGetPackageManagerVersion = jest
      .spyOn(getPackageManagerVersion, "default")
      .mockReturnValue(packageManagerVersion);
    const mockedGetWorkspaceImplementation = jest
      .spyOn(workspaceImplementation, "default")
      .mockReturnValue(packageManager);

    const packageJson = readJson("package.json");
    const turboJson = readJson("turbo.json");

    await migrate(root as MigrateCommandArgument, {
      force: false,
      dry: true,
      print: false,
      install: true,
    });

    // make sure nothing changed
    expect(readJson("package.json")).toStrictEqual(packageJson);
    expect(readJson("turbo.json")).toStrictEqual(turboJson);

    // verify mocks were called
    expect(mockedCheckGitStatus).not.toHaveBeenCalled();
    expect(mockedGetCurrentVersion).toHaveBeenCalled();
    expect(mockedGetLatestVersion).toHaveBeenCalled();
    expect(mockedGetTurboUpgradeCommand).toHaveBeenCalled();
    expect(mockedGetPackageManagerVersion).toHaveBeenCalled();
    expect(mockedGetWorkspaceImplementation).toHaveBeenCalled();

    // restore mocks
    mockedCheckGitStatus.mockRestore();
    mockedGetCurrentVersion.mockRestore();
    mockedGetLatestVersion.mockRestore();
    mockedGetTurboUpgradeCommand.mockRestore();
    mockedGetPackageManagerVersion.mockRestore();
    mockedGetWorkspaceImplementation.mockRestore();
  });

  it("next version can be passed as an option", async () => {
    const { root, readJson } = useFixture({
      fixture: "old-turbo",
    });

    const packageManager = "pnpm";
    const packageManagerVersion = "1.2.3";

    // setup mocks
    const mockedCheckGitStatus = jest
      .spyOn(checkGitStatus, "default")
      .mockReturnValue(undefined);
    const mockedGetCurrentVersion = jest
      .spyOn(getCurrentVersion, "default")
      .mockReturnValue("1.0.0");
    const mockedGetLatestVersion = jest
      .spyOn(getLatestVersion, "default")
      .mockResolvedValue("1.7.0");
    const mockedGetTurboUpgradeCommand = jest
      .spyOn(getTurboUpgradeCommand, "default")
      .mockReturnValue("pnpm install -g turbo@latest");
    const mockedGetPackageManagerVersion = jest
      .spyOn(getPackageManagerVersion, "default")
      .mockReturnValue(packageManagerVersion);
    const mockedGetWorkspaceImplementation = jest
      .spyOn(workspaceImplementation, "default")
      .mockReturnValue(packageManager);

    await migrate(root as MigrateCommandArgument, {
      force: false,
      dry: false,
      print: false,
      install: false,
      to: "1.7.0",
    });

    expect(readJson("package.json")).toStrictEqual({
      dependencies: {},
      devDependencies: {
        turbo: "1.0.0",
      },
      name: "no-turbo-json",
      packageManager: "pnpm@1.2.3",
      version: "1.0.0",
    });
    expect(readJson("turbo.json")).toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      pipeline: {
        build: {
          outputs: [".next/**", "!.next/cache/**"],
        },
        dev: {
          cache: false,
        },
        test: {
          outputs: ["dist/**", "build/**"],
        },
        lint: {},
      },
    });

    // verify mocks were called
    expect(mockedCheckGitStatus).toHaveBeenCalled();
    expect(mockedGetCurrentVersion).toHaveBeenCalled();
    expect(mockedGetLatestVersion).toHaveBeenCalled();
    expect(mockedGetTurboUpgradeCommand).toHaveBeenCalled();
    expect(mockedGetPackageManagerVersion).toHaveBeenCalled();
    expect(mockedGetWorkspaceImplementation).toHaveBeenCalled();

    // restore mocks
    mockedCheckGitStatus.mockRestore();
    mockedGetCurrentVersion.mockRestore();
    mockedGetLatestVersion.mockRestore();
    mockedGetTurboUpgradeCommand.mockRestore();
    mockedGetPackageManagerVersion.mockRestore();
    mockedGetWorkspaceImplementation.mockRestore();
  });

  it("current version can be passed as an option", async () => {
    const { root, readJson } = useFixture({
      fixture: "old-turbo",
    });

    const packageManager = "pnpm";
    const packageManagerVersion = "1.2.3";

    // setup mocks
    const mockedCheckGitStatus = jest
      .spyOn(checkGitStatus, "default")
      .mockReturnValue(undefined);
    const mockedGetLatestVersion = jest
      .spyOn(getLatestVersion, "default")
      .mockResolvedValue("1.7.0");
    const mockedGetTurboUpgradeCommand = jest
      .spyOn(getTurboUpgradeCommand, "default")
      .mockReturnValue("pnpm install -g turbo@latest");
    const mockedGetPackageManagerVersion = jest
      .spyOn(getPackageManagerVersion, "default")
      .mockReturnValue(packageManagerVersion);

    const mockedGetWorkspaceImplementation = jest
      .spyOn(workspaceImplementation, "default")
      .mockReturnValue(packageManager);

    await migrate(root as MigrateCommandArgument, {
      force: false,
      dry: false,
      print: false,
      install: false,
      from: "1.0.0",
    });

    expect(readJson("package.json")).toStrictEqual({
      dependencies: {},
      devDependencies: {
        turbo: "1.0.0",
      },
      name: "no-turbo-json",
      packageManager: "pnpm@1.2.3",
      version: "1.0.0",
    });
    expect(readJson("turbo.json")).toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      pipeline: {
        build: {
          outputs: [".next/**", "!.next/cache/**"],
        },
        dev: {
          cache: false,
        },
        lint: {},
        test: {
          outputs: ["dist/**", "build/**"],
        },
      },
    });

    // verify mocks were called
    expect(mockedCheckGitStatus).toHaveBeenCalled();
    expect(mockedGetLatestVersion).toHaveBeenCalled();
    expect(mockedGetTurboUpgradeCommand).toHaveBeenCalled();
    expect(mockedGetPackageManagerVersion).toHaveBeenCalled();
    expect(mockedGetWorkspaceImplementation).toHaveBeenCalled();

    // restore mocks
    mockedCheckGitStatus.mockRestore();
    mockedGetLatestVersion.mockRestore();
    mockedGetTurboUpgradeCommand.mockRestore();
    mockedGetPackageManagerVersion.mockRestore();
    mockedGetWorkspaceImplementation.mockRestore();
  });

  it("exits if the current version is the same as the new version", async () => {
    const { root } = useFixture({
      fixture: "old-turbo",
    });

    // setup mocks
    const mockedCheckGitStatus = jest
      .spyOn(checkGitStatus, "default")
      .mockReturnValue(undefined);
    const mockedGetCurrentVersion = jest
      .spyOn(getCurrentVersion, "default")
      .mockReturnValue("1.7.0");
    const mockedGetLatestVersion = jest
      .spyOn(getLatestVersion, "default")
      .mockResolvedValue("1.7.0");

    await migrate(root as MigrateCommandArgument, {
      force: false,
      dry: false,
      print: false,
      install: false,
    });

    expect(mockExit.exit).toHaveBeenCalledWith(0);

    // verify mocks were called
    expect(mockedCheckGitStatus).toHaveBeenCalled();
    expect(mockedGetCurrentVersion).toHaveBeenCalled();
    expect(mockedGetLatestVersion).toHaveBeenCalled();

    // restore mocks
    mockedCheckGitStatus.mockRestore();
    mockedGetCurrentVersion.mockRestore();
    mockedGetLatestVersion.mockRestore();
  });

  it("continues when migration doesn't require codemods", async () => {
    const { root } = useFixture({
      fixture: "old-turbo",
    });

    // setup mocks
    const mockedCheckGitStatus = jest
      .spyOn(checkGitStatus, "default")
      .mockReturnValue(undefined);
    const mockedGetCurrentVersion = jest
      .spyOn(getCurrentVersion, "default")
      .mockReturnValue("1.3.0");
    const mockedGetLatestVersion = jest
      .spyOn(getLatestVersion, "default")
      .mockResolvedValue("1.3.1");
    const mockedGetTurboUpgradeCommand = jest
      .spyOn(getTurboUpgradeCommand, "default")
      .mockReturnValue("npm install turbo@1.3.1");
    const mockExecSync = jest
      .spyOn(childProcess, "execSync")
      .mockReturnValue("installed");

    await migrate(root as MigrateCommandArgument, {
      force: false,
      dry: false,
      print: false,
      install: true,
    });

    // verify mocks were called
    expect(mockedCheckGitStatus).toHaveBeenCalled();
    expect(mockedGetCurrentVersion).toHaveBeenCalled();
    expect(mockedGetLatestVersion).toHaveBeenCalled();
    expect(mockedGetTurboUpgradeCommand).toHaveBeenCalled();
    expect(mockExecSync).toHaveBeenCalledWith("npm install turbo@1.3.1", {
      cwd: root,
    });

    // restore mocks
    mockedCheckGitStatus.mockRestore();
    mockedGetCurrentVersion.mockRestore();
    mockedGetLatestVersion.mockRestore();
    mockedGetTurboUpgradeCommand.mockRestore();
    mockExecSync.mockRestore();
  });

  it("installs the correct turbo version", async () => {
    const { root, readJson } = useFixture({
      fixture: "old-turbo",
    });

    const packageManager = "pnpm";
    const packageManagerVersion = "1.2.3";

    // setup mocks
    const mockedCheckGitStatus = jest
      .spyOn(checkGitStatus, "default")
      .mockReturnValue(undefined);
    const mockedGetCurrentVersion = jest
      .spyOn(getCurrentVersion, "default")
      .mockReturnValue("1.0.0");
    const mockedGetLatestVersion = jest
      .spyOn(getLatestVersion, "default")
      .mockResolvedValue("1.7.0");
    const mockedGetTurboUpgradeCommand = jest
      .spyOn(getTurboUpgradeCommand, "default")
      .mockReturnValue("pnpm install -g turbo@1.7.0");
    const mockedGetPackageManagerVersion = jest
      .spyOn(getPackageManagerVersion, "default")
      .mockReturnValue(packageManagerVersion);
    const mockedGetWorkspaceImplementation = jest
      .spyOn(workspaceImplementation, "default")
      .mockReturnValue(packageManager);
    const mockExecSync = jest
      .spyOn(childProcess, "execSync")
      .mockReturnValue("installed");

    await migrate(root as MigrateCommandArgument, {
      force: false,
      dry: false,
      print: false,
      install: true,
    });

    expect(readJson("package.json")).toStrictEqual({
      dependencies: {},
      devDependencies: {
        turbo: "1.0.0",
      },
      name: "no-turbo-json",
      packageManager: "pnpm@1.2.3",
      version: "1.0.0",
    });
    expect(readJson("turbo.json")).toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      pipeline: {
        build: {
          outputs: [".next/**", "!.next/cache/**"],
        },
        dev: {
          cache: false,
        },
        lint: {},
        test: {
          outputs: ["dist/**", "build/**"],
        },
      },
    });

    // verify mocks were called
    expect(mockedCheckGitStatus).toHaveBeenCalled();
    expect(mockedGetCurrentVersion).toHaveBeenCalled();
    expect(mockedGetLatestVersion).toHaveBeenCalled();
    expect(mockedGetTurboUpgradeCommand).toHaveBeenCalled();
    expect(mockedGetPackageManagerVersion).toHaveBeenCalled();
    expect(mockedGetWorkspaceImplementation).toHaveBeenCalled();
    expect(mockExecSync).toHaveBeenCalled();
    expect(mockExecSync).toHaveBeenCalledWith("pnpm install -g turbo@1.7.0", {
      cwd: root,
    });

    // restore mocks
    mockedCheckGitStatus.mockRestore();
    mockedGetCurrentVersion.mockRestore();
    mockedGetLatestVersion.mockRestore();
    mockedGetTurboUpgradeCommand.mockRestore();
    mockedGetPackageManagerVersion.mockRestore();
    mockedGetWorkspaceImplementation.mockRestore();
    mockExecSync.mockRestore();
  });

  it("fails gracefully when the correct upgrade command cannot be found", async () => {
    const { root, readJson } = useFixture({
      fixture: "old-turbo",
    });

    const packageManager = "pnpm";
    const packageManagerVersion = "1.2.3";

    // setup mocks
    const mockedCheckGitStatus = jest
      .spyOn(checkGitStatus, "default")
      .mockReturnValue(undefined);
    const mockedGetCurrentVersion = jest
      .spyOn(getCurrentVersion, "default")
      .mockReturnValue("1.0.0");
    const mockedGetLatestVersion = jest
      .spyOn(getLatestVersion, "default")
      .mockResolvedValue("1.7.0");
    const mockedGetTurboUpgradeCommand = jest
      .spyOn(getTurboUpgradeCommand, "default")
      .mockReturnValue(undefined);
    const mockedGetPackageManagerVersion = jest
      .spyOn(getPackageManagerVersion, "default")
      .mockReturnValue(packageManagerVersion);
    const mockedGetWorkspaceImplementation = jest
      .spyOn(workspaceImplementation, "default")
      .mockReturnValue(packageManager);
    const mockExecSync = jest
      .spyOn(childProcess, "execSync")
      .mockReturnValue("installed");

    await migrate(root as MigrateCommandArgument, {
      force: false,
      dry: false,
      print: false,
      install: true,
    });

    expect(readJson("package.json")).toStrictEqual({
      dependencies: {},
      devDependencies: {
        turbo: "1.0.0",
      },
      name: "no-turbo-json",
      packageManager: "pnpm@1.2.3",
      version: "1.0.0",
    });
    expect(readJson("turbo.json")).toStrictEqual({
      $schema: "https://turbo.build/schema.json",
      pipeline: {
        build: {
          outputs: [".next/**", "!.next/cache/**"],
        },
        dev: {
          cache: false,
        },
        lint: {},
        test: {
          outputs: ["dist/**", "build/**"],
        },
      },
    });

    expect(mockExit.exit).toHaveBeenCalledWith(1);

    // verify mocks were called
    expect(mockedCheckGitStatus).toHaveBeenCalled();
    expect(mockedGetCurrentVersion).toHaveBeenCalled();
    expect(mockedGetLatestVersion).toHaveBeenCalled();
    expect(mockedGetTurboUpgradeCommand).toHaveBeenCalled();
    expect(mockedGetPackageManagerVersion).toHaveBeenCalled();
    expect(mockedGetWorkspaceImplementation).toHaveBeenCalled();
    expect(mockExecSync).not.toHaveBeenCalled();

    // restore mocks
    mockedCheckGitStatus.mockRestore();
    mockedGetCurrentVersion.mockRestore();
    mockedGetLatestVersion.mockRestore();
    mockedGetTurboUpgradeCommand.mockRestore();
    mockedGetPackageManagerVersion.mockRestore();
    mockedGetWorkspaceImplementation.mockRestore();
    mockExecSync.mockRestore();
  });

  it("exits if current version is not passed and cannot be inferred", async () => {
    const { root } = useFixture({
      fixture: "old-turbo",
    });

    // setup mocks
    const mockedCheckGitStatus = jest
      .spyOn(checkGitStatus, "default")
      .mockReturnValue(undefined);
    const mockedGetCurrentVersion = jest
      .spyOn(getCurrentVersion, "default")
      .mockReturnValue(undefined);

    await migrate(root as MigrateCommandArgument, {
      force: false,
      dry: false,
      print: false,
      install: false,
    });

    expect(mockExit.exit).toHaveBeenCalledWith(1);

    // verify mocks were called
    expect(mockedCheckGitStatus).toHaveBeenCalled();
    expect(mockedGetCurrentVersion).toHaveBeenCalled();

    // restore mocks
    mockedCheckGitStatus.mockRestore();
    mockedGetCurrentVersion.mockRestore();
  });

  it("exits if latest version is not passed and cannot be inferred", async () => {
    const { root } = useFixture({
      fixture: "old-turbo",
    });

    // setup mocks
    const mockedCheckGitStatus = jest
      .spyOn(checkGitStatus, "default")
      .mockReturnValue(undefined);
    const mockedGetCurrentVersion = jest
      .spyOn(getCurrentVersion, "default")
      .mockReturnValue("1.5.0");
    const mockedGetLatestVersion = jest
      .spyOn(getLatestVersion, "default")
      .mockResolvedValue(undefined);

    await migrate(root as MigrateCommandArgument, {
      force: false,
      dry: false,
      print: false,
      install: false,
    });

    expect(mockExit.exit).toHaveBeenCalledWith(1);

    // verify mocks were called
    expect(mockedCheckGitStatus).toHaveBeenCalled();
    expect(mockedGetCurrentVersion).toHaveBeenCalled();
    expect(mockedGetLatestVersion).toHaveBeenCalled();

    // restore mocks
    mockedCheckGitStatus.mockRestore();
    mockedGetCurrentVersion.mockRestore();
    mockedGetLatestVersion.mockRestore();
  });

  it("exits if latest version throws", async () => {
    const { root } = useFixture({
      fixture: "old-turbo",
    });

    // setup mocks
    const mockedCheckGitStatus = jest
      .spyOn(checkGitStatus, "default")
      .mockReturnValue(undefined);
    const mockedGetCurrentVersion = jest
      .spyOn(getCurrentVersion, "default")
      .mockReturnValue("1.5.0");
    const mockedGetLatestVersion = jest
      .spyOn(getLatestVersion, "default")
      .mockRejectedValue(new Error("failed to fetch version"));

    await migrate(root as MigrateCommandArgument, {
      force: false,
      dry: false,
      print: false,
      install: false,
    });

    expect(mockExit.exit).toHaveBeenCalledWith(1);

    // verify mocks were called
    expect(mockedCheckGitStatus).toHaveBeenCalled();
    expect(mockedGetCurrentVersion).toHaveBeenCalled();
    expect(mockedGetLatestVersion).toHaveBeenCalled();

    // restore mocks
    mockedCheckGitStatus.mockRestore();
    mockedGetCurrentVersion.mockRestore();
    mockedGetLatestVersion.mockRestore();
  });

  it("exits if any transforms encounter an error", async () => {
    const { root } = useFixture({
      fixture: "old-turbo",
    });

    const packageManager = "pnpm";
    const packageManagerVersion = "1.2.3";

    // setup mocks
    const mockedCheckGitStatus = jest
      .spyOn(checkGitStatus, "default")
      .mockReturnValue(undefined);
    const mockedGetCurrentVersion = jest
      .spyOn(getCurrentVersion, "default")
      .mockReturnValue("1.0.0");
    const mockedGetLatestVersion = jest
      .spyOn(getLatestVersion, "default")
      .mockResolvedValue("1.7.0");
    const mockedGetPackageManagerVersion = jest
      .spyOn(getPackageManagerVersion, "default")
      .mockReturnValue(packageManagerVersion);
    const mockedGetWorkspaceImplementation = jest
      .spyOn(workspaceImplementation, "default")
      .mockReturnValue(packageManager);

    await migrate(root as MigrateCommandArgument, {
      force: false,
      dry: true,
      print: false,
      install: true,
    });

    expect(mockExit.exit).toHaveBeenCalledWith(1);

    // verify mocks were called
    expect(mockedCheckGitStatus).not.toHaveBeenCalled();
    expect(mockedGetCurrentVersion).toHaveBeenCalled();
    expect(mockedGetLatestVersion).toHaveBeenCalled();
    expect(mockedGetPackageManagerVersion).toHaveBeenCalled();
    expect(mockedGetWorkspaceImplementation).toHaveBeenCalled();

    // restore mocks
    mockedCheckGitStatus.mockRestore();
    mockedGetCurrentVersion.mockRestore();
    mockedGetLatestVersion.mockRestore();
    mockedGetPackageManagerVersion.mockRestore();
    mockedGetWorkspaceImplementation.mockRestore();
  });

  it("exits if invalid directory is passed", async () => {
    const { root } = useFixture({
      fixture: "old-turbo",
    });

    // setup mocks
    const mockedCheckGitStatus = jest
      .spyOn(checkGitStatus, "default")
      .mockReturnValue(undefined);

    await migrate("~/path/that/does/not/exist" as MigrateCommandArgument, {
      force: false,
      dry: false,
      print: false,
      install: false,
    });

    expect(mockExit.exit).toHaveBeenCalledWith(1);

    // verify mocks were called
    expect(mockedCheckGitStatus).toHaveBeenCalled();

    // restore mocks
    mockedCheckGitStatus.mockRestore();
  });

  it("exits if directory with no repo is passed", async () => {
    const { root } = useFixture({
      fixture: "no-repo",
    });

    // setup mocks
    const mockedCheckGitStatus = jest
      .spyOn(checkGitStatus, "default")
      .mockReturnValue(undefined);

    await migrate(root as MigrateCommandArgument, {
      force: false,
      dry: false,
      print: false,
      install: false,
    });

    expect(mockExit.exit).toHaveBeenCalledWith(1);

    // verify mocks were called
    expect(mockedCheckGitStatus).toHaveBeenCalled();

    // restore mocks
    mockedCheckGitStatus.mockRestore();
  });
});
