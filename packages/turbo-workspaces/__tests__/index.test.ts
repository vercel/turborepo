import path from "node:path";
import execa from "execa";
import * as turboUtils from "@turbo/utils";
import { setupTestFixtures } from "@turbo/test-utils";
import { describe, it, expect, jest, beforeEach } from "@jest/globals";
import { getWorkspaceDetails, convert, install } from "../src";
import { generateConvertMatrix } from "./test-utils";

jest.mock("execa", () => jest.fn());

describe("Node entrypoint", () => {
  const { useFixture } = setupTestFixtures({
    directory: path.join(__dirname, "../")
  });

  beforeEach(() => {
    jest.clearAllMocks();
    (execa as jest.MockedFunction<typeof execa>).mockResolvedValue({
      stdout: "",
      stderr: "",
      exitCode: 0,
      command: "",
      failed: false,
      timedOut: false,
      isCanceled: false,
      killed: false
    } as any);
  });

  describe("install", () => {
    it("should use shell option on Windows for all package managers", async () => {
      const originalPlatform = process.platform;
      Object.defineProperty(process, "platform", {
        value: "win32"
      });

      const { root } = useFixture({
        fixture: `./bun/monorepo`
      });

      const mockProject = {
        name: "test-project",
        description: undefined,
        packageManager: "bun" as const,
        paths: {
          root,
          packageJson: path.join(root, "package.json"),
          lockfile: path.join(root, "bun.lockb"),
          nodeModules: path.join(root, "node_modules")
        },
        workspaceData: {
          globs: ["apps/*", "packages/*"],
          workspaces: []
        }
      };

      await install({
        project: mockProject,
        to: { name: "bun", version: "1.0.1" },
        options: { dry: false }
      });

      expect(execa).toHaveBeenCalledWith("bun", ["install"], {
        cwd: root,
        preferLocal: true,
        shell: true,
        stdin: "ignore"
      });

      Object.defineProperty(process, "platform", {
        value: originalPlatform
      });
    });

    it("should not use shell option on non-Windows platforms", async () => {
      const originalPlatform = process.platform;
      Object.defineProperty(process, "platform", {
        value: "darwin"
      });

      const { root } = useFixture({
        fixture: `./bun/monorepo`
      });

      const mockProject = {
        name: "test-project",
        description: undefined,
        packageManager: "bun" as const,
        paths: {
          root,
          packageJson: path.join(root, "package.json"),
          lockfile: path.join(root, "bun.lockb"),
          nodeModules: path.join(root, "node_modules")
        },
        workspaceData: {
          globs: ["apps/*", "packages/*"],
          workspaces: []
        }
      };

      await install({
        project: mockProject,
        to: { name: "bun", version: "1.0.1" },
        options: { dry: false }
      });

      expect(execa).toHaveBeenCalledWith("bun", ["install"], {
        cwd: root,
        preferLocal: true,
        shell: false,
        stdin: "ignore"
      });

      Object.defineProperty(process, "platform", {
        value: originalPlatform
      });
    });

    it.each([
      {
        manager: "npm" as const,
        version: "8.19.2",
        lockfile: "package-lock.json",
        installArgs: ["install"]
      },
      {
        manager: "pnpm" as const,
        version: "7.29.1",
        lockfile: "pnpm-lock.yaml",
        installArgs: ["install", "--fix-lockfile"]
      },
      {
        manager: "yarn" as const,
        version: "1.22.19",
        lockfile: "yarn.lock",
        installArgs: ["install"]
      },
      {
        manager: "bun" as const,
        version: "1.0.1",
        lockfile: "bun.lockb",
        installArgs: ["install"]
      }
    ])(
      "should use stdin: ignore for $manager to prevent hanging in non-interactive environments",
      async ({ manager, version, lockfile, installArgs }) => {
        const { root } = useFixture({
          fixture: `./${manager}/monorepo`
        });

        const mockProject = {
          name: "test-project",
          description: undefined,
          packageManager: manager,
          paths: {
            root,
            packageJson: path.join(root, "package.json"),
            lockfile: path.join(root, lockfile),
            nodeModules: path.join(root, "node_modules")
          },
          workspaceData: {
            globs: ["apps/*", "packages/*"],
            workspaces: []
          }
        };

        await install({
          project: mockProject,
          to: { name: manager, version },
          options: { dry: false }
        });

        expect(execa).toHaveBeenCalledWith(
          manager,
          installArgs,
          expect.objectContaining({
            stdin: "ignore"
          })
        );
      }
    );
  });

  describe("convert", () => {
    it.each(generateConvertMatrix())(
      "detects $fixtureType project using $fixtureManager and converts to $toManager (interactive=$interactive dry=$dry install=$install)",
      async ({
        fixtureManager,
        fixtureType,
        toManager,
        interactive,
        dry,
        install
      }) => {
        const mockedGetAvailablePackageManagers = jest
          .spyOn(turboUtils, "getAvailablePackageManagers")
          .mockResolvedValue({
            npm: "8.19.2",
            yarn: "1.22.19",
            pnpm: "7.29.1",
            bun: "1.0.1"
          });

        const { root } = useFixture({
          fixture: `./${fixtureManager}/${fixtureType}`
        });

        // read
        const details = await getWorkspaceDetails({ root });
        expect(details.packageManager).toBe(fixtureManager);

        // convert
        const convertWrapper = () =>
          convert({
            root,
            to: toManager,
            options: { interactive, dry, skipInstall: !install }
          });

        if (fixtureManager === toManager) {
          await expect(convertWrapper()).rejects.toThrowError(
            "You are already using this package manager"
          );
        } else {
          await expect(convertWrapper()).resolves.toBeUndefined();
          // read again
          const convertedDetails = await getWorkspaceDetails({
            root
          });
          expect(mockedGetAvailablePackageManagers).toHaveBeenCalled();

          if (dry) {
            expect(convertedDetails.packageManager).toBe(fixtureManager);
          } else {
            if (install) {
              expect(execa).toHaveBeenCalled();
            }
            expect(convertedDetails.packageManager).toBe(toManager);
          }
        }

        mockedGetAvailablePackageManagers.mockRestore();
      }
    );
  });
});
