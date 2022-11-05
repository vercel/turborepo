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

const yarn = MANAGERS.yarn;

describe("yarn", () => {
  const { useFixture } = setupTestFixtures({
    directory: path.join(__dirname, "../../"),
    test: "yarn",
  });

  describe("detect", () => {
    test.each([
      ["npm", false],
      ["yarn", true],
      ["pnpm", false],
    ])("detects yarn workspaces from %s workspaces", async (from, result) => {
      const { root } = useFixture({ fixture: `../${from}/basic` });

      expect(
        await yarn.detect({
          workspaceRoot: root,
        })
      ).toEqual(result);
    });
  });

  describe("remove", () => {
    test.each(
      extendMatrix([
        ["npm", "8.19.2", ["apps/*", "packages/*"]],
        ["yarn", "1.22.19", undefined],
        ["pnpm", "7.12.1", undefined],
      ])
    )(
      "removes yarn workspaces when moving to %s@%s | workspaces=%s withNodeModules=%s interactive=%s and dry=%s",
      async (to, version, workspaces, withNodeModules, interactive, dry) => {
        // start with yarn
        const { root, readJson } = useFixture({ fixture: `basic` });
        if (withNodeModules) {
          fs.ensureDirSync(path.join(root, "node_modules"));
        }
        const project = await yarn.read({
          workspaceRoot: root,
        });

        // remove yarn
        await yarn.remove({
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

  describe("create", () => {
    test.each(generateTestMatrix())(
      "creates yarn workspaces from %s workspaces with interactive=%s and dry=%s",
      async (from, interactive, dry) => {
        const { root } = useFixture({ fixture: `../${from}/basic` });
        const project = await MANAGERS[from].read({
          workspaceRoot: root,
        });

        // convert to yarn
        await yarn.create({
          project,
          to: { name: "yarn", version: "1.22.19" },
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
          expect(await yarn.detect({ workspaceRoot: root })).toEqual(true);
        }
      }
    );
  });

  describe("read", () => {
    test.each<[PackageManagers, boolean]>([
      ["yarn", false],
      ["npm", true],
      ["pnpm", true],
    ])(
      "reads yarn workspaces from %s workspaces (should throw=%s)",
      async (from, shouldThrow) => {
        const { root, tmpDirectory } = useFixture({
          fixture: `../${from}/basic`,
        });

        const read = async () => yarn.read({ workspaceRoot: path.join(root) });
        if (shouldThrow) {
          expect(read).rejects.toThrow("Not a yarn workspaces project");
          return;
        }
        const project = await yarn.read({
          workspaceRoot: path.join(root),
        });

        expect(project.name).toEqual("yarn-workspaces");
        expect(project.packageManager).toEqual("yarn");
        // paths
        expect(project.paths.root).toMatch(
          new RegExp(`^.*yarn\/${tmpDirectory}$`)
        );
        expect(project.paths.lockfile).toMatch(
          new RegExp(`^.*yarn\/${tmpDirectory}\/yarn.lock$`)
        );
        expect(project.paths.packageJson).toMatch(
          new RegExp(`^.*yarn\/${tmpDirectory}\/package.json$`)
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
      "converts %s workspaces lockfile to yarn lockfile",
      async (from) => {
        const { root } = useFixture({ fixture: `../${from}/basic` });
        const project = await MANAGERS[from].read({
          workspaceRoot: root,
        });

        expect(
          await yarn.convertLock({
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
    ])("cleans %s yarn workspaces", async (interactive, dry) => {
      const { root } = useFixture({ fixture: "basic" });
      const project = await yarn.read({
        workspaceRoot: root,
      });

      await yarn.clean({
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
