import transform from "../src/commands/transform";
import { MigrateCommandArgument } from "../src/commands";
import { setupTestFixtures, spyExit } from "@turbo/test-utils";
import * as checkGitStatus from "../src/utils/checkGitStatus";
import * as getPackageManager from "../src/utils/getPackageManager";
import * as getPackageManagerVersion from "../src/utils/getPackageManagerVersion";

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
      .spyOn(checkGitStatus, "default")
      .mockReturnValue(undefined);
    const mockedGetPackageManagerVersion = jest
      .spyOn(getPackageManagerVersion, "default")
      .mockReturnValue(packageManagerVersion);
    const mockedGetPackageManager = jest
      .spyOn(getPackageManager, "default")
      .mockReturnValue(packageManager);

    await transform(
      "add-package-manager" as MigrateCommandArgument,
      root as MigrateCommandArgument,
      {
        list: false,
        force: false,
        dry: false,
        print: false,
      }
    );

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
    expect(mockedGetPackageManagerVersion).toHaveBeenCalled();
    expect(mockedGetPackageManager).toHaveBeenCalled();

    // restore mocks
    mockedCheckGitStatus.mockRestore();
    mockedGetPackageManagerVersion.mockRestore();
    mockedGetPackageManager.mockRestore();
  });

  it("runs the selected transform - dry & print", async () => {
    const { root, readJson } = useFixture({
      fixture: "basic",
    });

    const packageManager = "pnpm";
    const packageManagerVersion = "1.2.3";

    // setup mocks
    const mockedCheckGitStatus = jest
      .spyOn(checkGitStatus, "default")
      .mockReturnValue(undefined);
    const mockedGetPackageManagerVersion = jest
      .spyOn(getPackageManagerVersion, "default")
      .mockReturnValue(packageManagerVersion);
    const mockedGetPackageManager = jest
      .spyOn(getPackageManager, "default")
      .mockReturnValue(packageManager);

    await transform(
      "add-package-manager" as MigrateCommandArgument,
      root as MigrateCommandArgument,
      {
        list: false,
        force: false,
        dry: true,
        print: true,
      }
    );

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
    expect(mockedGetPackageManagerVersion).toHaveBeenCalled();
    expect(mockedGetPackageManager).toHaveBeenCalled();

    // restore mocks
    mockedCheckGitStatus.mockRestore();
    mockedGetPackageManagerVersion.mockRestore();
    mockedGetPackageManager.mockRestore();
  });

  it("lists transforms", async () => {
    const { root } = useFixture({
      fixture: "basic",
    });

    await transform(
      "add-package-manager" as MigrateCommandArgument,
      root as MigrateCommandArgument,
      {
        list: true,
        force: false,
        dry: false,
        print: false,
      }
    );

    expect(mockExit.exit).toHaveBeenCalledWith(0);
  });

  it("exits on invalid transform", async () => {
    const { root } = useFixture({
      fixture: "basic",
    });

    await transform(
      "not-a-real-option" as MigrateCommandArgument,
      root as MigrateCommandArgument,
      {
        list: false,
        force: false,
        dry: false,
        print: false,
      }
    );

    expect(mockExit.exit).toHaveBeenCalledWith(1);
  });

  it("exits on invalid directory", async () => {
    const { root } = useFixture({
      fixture: "basic",
    });

    await transform(
      "add-package-manager" as MigrateCommandArgument,
      "~/path/that/does/not/exist" as MigrateCommandArgument,
      {
        list: false,
        force: false,
        dry: false,
        print: false,
      }
    );

    expect(mockExit.exit).toHaveBeenCalledWith(1);
  });
});
