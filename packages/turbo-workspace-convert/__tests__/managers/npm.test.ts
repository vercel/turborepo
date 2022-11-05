import path from "path";
import { setupTestFixtures } from "turbo-test-utils";
import { Logger } from "../../src/logger";
import MANAGERS from "../../src/managers";
import { PackageManagers } from "../../src/types";
import fs from "fs-extra";
import {
  extendMatrix,
  generateTestMatrix,
  validateWorkspace,
} from "../test-utils";

const npm = MANAGERS.npm;

describe("npm", () => {
  const { useFixture } = setupTestFixtures({
    directory: path.join(__dirname, "../../"),
    test: "npm",
  });

  describe("detect", () => {
    test.each([
      ["npm", true],
      ["yarn", false],
      ["pnpm", false],
    ])("detects npm workspaces from %s workspaces", async (from, result) => {
      const { root } = useFixture({ fixture: `../${from}/basic` });

      expect(
        await npm.detect({
          workspaceRoot: root,
        })
      ).toEqual(result);
    });
  });

  describe("create", () => {
    test.each(generateTestMatrix())(
      "creates yarn workspaces from %s workspaces with interactive=%s and dry=%s",
      async (from, interactive, dry) => {
        const { root } = useFixture({ fixture: `../${from}/basic` });
        const project = await MANAGERS[from].read({
          workspaceRoot: root,
        });

        // convert to npm
        await npm.create({
          project,
          to: { name: "npm", version: "8.19.2" },
          logger: new Logger({ interactive, dry }),
          options: {
            interactive,
            dry,
          },
        });

        if (dry) {
          expect(await MANAGERS[from].detect({ workspaceRoot: root })).toEqual(
            true
          );
        } else {
          expect(await npm.detect({ workspaceRoot: root })).toEqual(true);
        }
      }
    );
  });

  describe("remove", () => {
    test.each(
      extendMatrix([
        ["npm", "8.19.2", undefined],
        ["yarn", "1.22.19", ["apps/*", "packages/*"]],
        ["pnpm", "7.12.1", undefined],
      ])
    )(
      "removes npm workspaces when moving to %s@%s | workspaces=%s withNodeModules=%s interactive=%s and dry=%s",
      async (to, version, workspaces, withNodeModules, interactive, dry) => {
        const { root, readJson } = useFixture({ fixture: "basic" });
        if (withNodeModules) {
          fs.ensureDirSync(path.join(root, "node_modules"));
        }

        const project = await npm.read({
          workspaceRoot: root,
        });

        await npm.remove({
          project,
          to: { name: to, version },
          logger: new Logger({ interactive, dry }),
          options: {
            interactive,
            dry,
          },
        });

        if (dry) {
          expect(readJson(project.paths.packageJson).workspaces).toEqual(
            project.workspaceData.globs
          );
        } else {
          expect(readJson(project.paths.packageJson).workspaces).toEqual(
            workspaces
          );
        }
      }
    );
  });

  describe("read", () => {
    test.each<[PackageManagers, boolean]>([
      ["yarn", true],
      ["npm", false],
      ["pnpm", true],
    ])(
      "reads npm workspaces from %s workspaces (should throw=%s)",
      async (from, shouldThrow) => {
        const { root, tmpDirectory } = useFixture({
          fixture: `../${from}/basic`,
        });

        const read = async () => npm.read({ workspaceRoot: path.join(root) });
        if (shouldThrow) {
          expect(read).rejects.toThrow("Not an npm workspaces project");
          return;
        }
        const project = await npm.read({
          workspaceRoot: path.join(root),
        });
        expect(project.name).toEqual("npm-workspaces");
        expect(project.packageManager).toEqual("npm");
        // paths
        expect(project.paths.root).toMatch(
          new RegExp(`^.*npm\/${tmpDirectory}$`)
        );
        expect(project.paths.lockfile).toMatch(
          new RegExp(`^.*npm\/${tmpDirectory}\/package-lock.json$`)
        );
        expect(project.paths.packageJson).toMatch(
          new RegExp(`^.*npm\/${tmpDirectory}\/package.json$`)
        );
        // workspaceData
        expect(project.workspaceData.globs).toEqual(["apps/*", "packages/*"]);
        expect(project.workspaceData.workspaces).toHaveLength(4);
        project.workspaceData.workspaces.forEach((workspace) =>
          validateWorkspace(tmpDirectory, workspace)
        );
      }
    );
  });

  describe("convertLock", () => {
    test.each<[PackageManagers]>([["npm"], ["yarn"], ["pnpm"]])(
      "converts %s workspaces lockfile to npm lockfile",
      async (from) => {
        const { root } = useFixture({ fixture: `../${from}/basic` });
        const project = await MANAGERS[from].read({
          workspaceRoot: root,
        });

        expect(
          await npm.convertLock({
            project,
            logger: new Logger(),
            options: {
              interactive: false,
              dry: false,
            },
          })
        ).toBeUndefined();
      }
    );
  });

  describe("clean", () => {
    test.each([
      [true, true],
      [false, true],
      [false, true],
      [true, false],
    ])("cleans %s npm workspaces", async (interactive, dry) => {
      const { root } = useFixture({ fixture: "basic" });
      const project = await npm.read({
        workspaceRoot: root,
      });

      await npm.clean({
        project,
        logger: new Logger({ interactive, dry }),
        options: {
          interactive,
          dry,
        },
      });

      if (dry) {
        expect(fs.existsSync(project.paths.lockfile)).toEqual(true);
      } else {
        expect(fs.existsSync(project.paths.lockfile)).toEqual(false);
      }
    });
  });
});
