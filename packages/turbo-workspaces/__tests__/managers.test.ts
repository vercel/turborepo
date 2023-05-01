import path from "path";
import { setupTestFixtures } from "@turbo/test-utils";
import { Logger } from "../src/logger";
import MANAGERS from "../src/managers";
import { PackageJson } from "../src/types";
import fs from "fs-extra";
import {
  generateDetectMatrix,
  generateCreateMatrix,
  generateRemoveMatrix,
  generateReadMatrix,
  generateCleanMatrix,
  generateConvertLockMatrix,
} from "./test-utils";

jest.mock("execa", () => jest.fn());

describe("managers", () => {
  const { useFixture } = setupTestFixtures({
    directory: path.join(__dirname, "../"),
  });

  describe("detect", () => {
    test.each(generateDetectMatrix())(
      "$project $type project detected by $manager manager - (expect: $result)",
      async ({ project, manager, type, result }) => {
        const { root } = useFixture({ fixture: `./${project}/${type}` });

        const detectResult = await MANAGERS[manager].detect({
          workspaceRoot: root,
        });

        expect(detectResult).toEqual(result);
      }
    );
  });

  describe("create", () => {
    test.each(generateCreateMatrix())(
      "creates $manager project from $project $type project (interactive=$interactive, dry=$dry)",
      async ({ project, manager, type, interactive, dry }) => {
        expect.assertions(2);

        const { root } = useFixture({ fixture: `./${project}/${type}` });
        const testProject = await MANAGERS[project].read({
          workspaceRoot: root,
        });

        expect(testProject.packageManager).toEqual(project);

        await MANAGERS[manager].create({
          project: testProject,
          to: { name: manager, version: "1.2.3" },
          logger: new Logger({ interactive, dry }),
          options: {
            interactive,
            dry,
          },
        });

        if (dry) {
          expect(
            await MANAGERS[project].detect({ workspaceRoot: root })
          ).toEqual(true);
        } else {
          expect(
            await MANAGERS[manager].detect({ workspaceRoot: root })
          ).toEqual(true);
        }
      }
    );
  });

  describe("remove", () => {
    test.each(generateRemoveMatrix())(
      "removes $fixtureManager from $fixtureManager $fixtureType project when moving to $toManager (withNodeModules=$withNodeModules, interactive=$interactive, dry=$dry)",
      async ({
        fixtureManager,
        fixtureType,
        toManager,
        withNodeModules,
        interactive,
        dry,
      }) => {
        const { root, readJson, readYaml } = useFixture({
          fixture: `./${fixtureManager}/${fixtureType}`,
        });
        const project = await MANAGERS[fixtureManager].read({
          workspaceRoot: root,
        });
        expect(project.packageManager).toEqual(fixtureManager);

        if (withNodeModules) {
          fs.ensureDirSync(project.paths.nodeModules);
        }

        await MANAGERS[fixtureManager].remove({
          project,
          to: { name: toManager, version: "1.2.3" },
          logger: new Logger({ interactive, dry }),
          options: {
            interactive,
            dry,
          },
        });

        if (withNodeModules) {
          expect(fs.existsSync(project.paths.nodeModules)).toEqual(dry);
        }

        const packageJson = readJson<PackageJson>(project.paths.packageJson);
        if (dry) {
          expect(packageJson?.packageManager).toBeDefined();
          expect(packageJson?.packageManager?.split("@")[0]).toEqual(
            fixtureManager
          );
          if (fixtureType === "basic") {
            if (fixtureManager === "pnpm") {
              expect(project.paths.workspaceConfig).toBeDefined();
              if (project.paths.workspaceConfig) {
                const workspaceConfig = readYaml<{ packages: Array<string> }>(
                  project.paths.workspaceConfig
                );
                expect(workspaceConfig?.packages).toBeDefined();
                expect(workspaceConfig?.packages).toEqual(
                  project.workspaceData.globs
                );
              }
            } else {
              expect(packageJson?.workspaces).toBeDefined();
              expect(packageJson?.workspaces).toEqual(
                project.workspaceData.globs
              );
            }
          }
        } else {
          expect(packageJson?.packageManager).toBeUndefined();
          if (fixtureType === "basic") {
            expect(packageJson?.workspaces).toBeUndefined();

            if (fixtureManager === "pnpm") {
              expect(project.paths.workspaceConfig).toBeDefined();
              if (project.paths.workspaceConfig) {
                const workspaceConfig = readYaml<{ packages: Array<string> }>(
                  project.paths.workspaceConfig
                );
                expect(workspaceConfig).toBeUndefined();
              }
            }
          }
        }
      }
    );
  });

  describe("read", () => {
    test.each(generateReadMatrix())(
      "reads $toManager workspaces from $fixtureManager $fixtureType project - (shouldThrow: $shouldThrow)",
      async ({ fixtureManager, fixtureType, toManager, shouldThrow }) => {
        const { root, directoryName } = useFixture({
          fixture: `./${fixtureManager}/${fixtureType}`,
        });

        const read = async () =>
          MANAGERS[toManager].read({ workspaceRoot: path.join(root) });
        if (shouldThrow) {
          if (toManager === "pnpm") {
            expect(read).rejects.toThrow(`Not a pnpm project`);
          } else if (toManager === "yarn") {
            expect(read).rejects.toThrow(`Not a yarn project`);
          } else if (toManager === "npm") {
            expect(read).rejects.toThrow(`Not an npm project`);
          }
          return;
        }
        const project = await MANAGERS[toManager].read({
          workspaceRoot: path.join(root),
        });

        expect(project.name).toEqual(
          fixtureType === "monorepo" ? `${toManager}-workspaces` : toManager
        );
        expect(project.packageManager).toEqual(toManager);

        // paths
        expect(project.paths.root).toMatch(
          new RegExp(`^.*\/${directoryName}$`)
        );
        expect(project.paths.packageJson).toMatch(
          new RegExp(`^.*\/${directoryName}\/package.json$`)
        );

        if (fixtureManager === "pnpm") {
          new RegExp(`^.*\/${directoryName}\/pnpm-lock.yaml$`);
        } else if (fixtureManager === "yarn") {
          new RegExp(`^.*\/${directoryName}\/yarn.lock$`);
        } else if (fixtureManager === "npm") {
          new RegExp(`^.*\/${directoryName}\/package-lock.json$`);
        } else {
          throw new Error("Invalid fixtureManager");
        }

        if (fixtureType === "non-monorepo") {
          expect(project.workspaceData.workspaces).toEqual([]);
          expect(project.workspaceData.globs).toEqual([]);
        } else {
          expect(project.workspaceData.globs).toEqual(["apps/*", "packages/*"]);
          project.workspaceData.workspaces.forEach((workspace) => {
            const type = ["web", "docs"].includes(workspace.name)
              ? "apps"
              : "packages";
            expect(workspace.paths.packageJson).toMatch(
              new RegExp(
                `^.*${directoryName}\/${type}\/${workspace.name}\/package.json$`
              )
            );
            expect(workspace.paths.root).toMatch(
              new RegExp(`^.*${directoryName}\/${type}\/${workspace.name}$`)
            );
          });
        }
      }
    );
  });

  describe("clean", () => {
    test.each(generateCleanMatrix())(
      "cleans $fixtureManager $fixtureType project (interactive=$interactive, dry=$dry)",
      async ({ fixtureManager, fixtureType, interactive, dry }) => {
        const { root } = useFixture({
          fixture: `./${fixtureManager}/${fixtureType}`,
        });

        const project = await MANAGERS[fixtureManager].read({
          workspaceRoot: root,
        });

        expect(project.packageManager).toEqual(fixtureManager);

        await MANAGERS[fixtureManager].clean({
          project,
          logger: new Logger({ interactive, dry }),
          options: {
            interactive,
            dry,
          },
        });

        expect(fs.existsSync(project.paths.lockfile)).toEqual(dry);
      }
    );
  });

  describe("convertLock", () => {
    test.each(generateConvertLockMatrix())(
      "converts lockfile for $fixtureManager $fixtureType project to $toManager format (interactive=$interactive, dry=$dry)",
      async ({ fixtureManager, fixtureType, toManager, interactive, dry }) => {
        const { root, exists } = useFixture({
          fixture: `./${fixtureManager}/${fixtureType}`,
        });

        const project = await MANAGERS[fixtureManager].read({
          workspaceRoot: root,
        });

        expect(project.packageManager).toEqual(fixtureManager);

        await MANAGERS[toManager].convertLock({
          project,
          logger: new Logger(),
          options: {
            interactive,
            dry,
          },
        });

        if (fixtureManager !== toManager) {
          expect(exists(project.paths.lockfile)).toEqual(dry);
        } else {
          expect(exists(project.paths.lockfile)).toEqual(true);
        }
      }
    );
  });
});
