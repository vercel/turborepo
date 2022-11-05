import path from "path";
import { setupTestFixtures } from "turbo-test-utils";
import { Logger } from "../../src/logger";
import MANAGERS from "../../src/managers";
import { PackageManagers } from "../../src/types";
import {
  extendMatrix,
  generateTestMatrix,
  validateWorkspace,
} from "../test-utils";
import fs from "fs-extra";
import { existsSync } from "fs-extra";

const pnpm = MANAGERS.pnpm;

describe("pnpm", () => {
  const { useFixture } = setupTestFixtures({
    directory: path.join(__dirname, "../../"),
    test: "pnpm",
  });

  describe("detect", () => {
    test.each([
      ["npm", false],
      ["yarn", false],
      ["pnpm", true],
    ])("detects pnpm workspaces from %s workspaces", async (from, result) => {
      const { root } = useFixture({ fixture: `../${from}/basic` });

      expect(
        await pnpm.detect({
          workspaceRoot: root,
        })
      ).toEqual(result);
    });
  });

  describe("remove", () => {
    test.each(
      extendMatrix([
        ["npm", "8.19.2", undefined],
        ["yarn", "1.22.19", undefined],
        ["pnpm", "7.12.1", undefined],
      ])
    )(
      "removes pnpm workspaces when moving to %s@%s | workspaces=%s withNodeModules=%s interactive=%s and dry=%s",
      async (to, version, workspaces, withNodeModules, interactive, dry) => {
        // start with pnpm
        const { root, readYaml } = useFixture({ fixture: `basic` });
        if (withNodeModules) {
          fs.ensureDirSync(path.join(root, "node_modules"));
        }
        const project = await pnpm.read({
          workspaceRoot: root,
        });

        // remove yarn
        await pnpm.remove({
          project,
          to: { name: to, version },
          logger: new Logger({ interactive, dry }),
          options: {
            interactive,
            dry,
          },
        });

        if (dry) {
          expect(project.paths.workspaceConfig).toBeDefined();
          if (project.paths.workspaceConfig) {
            const workspaceConfig = readYaml(
              project.paths.workspaceConfig
            ) as Record<string, Array<string>>;
            expect(workspaceConfig.packages).toEqual(
              project.workspaceData.globs
            );
          }
        } else {
          expect(project.paths.workspaceConfig).toBeDefined();
          if (project.paths.workspaceConfig) {
            expect(readYaml(project.paths.workspaceConfig)).toEqual(workspaces);
          }
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

        // convert to pnpm
        await pnpm.create({
          project,
          to: { name: "pnpm", version: "7.27.0" },
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
          expect(await pnpm.detect({ workspaceRoot: root })).toEqual(true);
        }
      }
    );
  });

  describe("read", () => {
    test.each<[PackageManagers, boolean]>([
      ["yarn", true],
      ["npm", true],
      ["pnpm", false],
    ])(
      "reads pnpm workspaces from %s workspaces (should throw=%s)",
      async (from, shouldThrow) => {
        const { root, tmpDirectory } = useFixture({
          fixture: `../${from}/basic`,
        });

        const read = async () => pnpm.read({ workspaceRoot: path.join(root) });
        if (shouldThrow) {
          expect(read).rejects.toThrow("Not a pnpm workspaces project");
          return;
        }
        const project = await pnpm.read({
          workspaceRoot: path.join(root),
        });
        expect(project.name).toEqual("pnpm-workspaces");
        expect(project.packageManager).toEqual("pnpm");
        // paths
        expect(project.paths.root).toMatch(
          new RegExp(`^.*pnpm\/${tmpDirectory}$`)
        );
        expect(project.paths.lockfile).toMatch(
          new RegExp(`^.*pnpm\/${tmpDirectory}\/pnpm-lock.yaml$`)
        );
        expect(project.paths.packageJson).toMatch(
          new RegExp(`^.*pnpm\/${tmpDirectory}\/package.json$`)
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
      "converts %s workspaces lockfile to pnpm lockfile",
      async (from) => {
        const { root } = useFixture({ fixture: `../${from}/basic` });
        const project = await MANAGERS[from].read({
          workspaceRoot: root,
        });
        const interactive = false;
        const dry = false;
        await pnpm.create({
          project,
          to: { name: "pnpm", version: "7.27.0" },
          logger: new Logger({ interactive, dry }),
          options: {
            interactive,
            dry,
          },
        });

        expect(existsSync(project.paths.lockfile)).toBe(true);

        await pnpm.convertLock({
          project,
          logger: new Logger(),
          options: {
            interactive,
            dry,
          },
        });

        expect(
          existsSync(path.join(project.paths.root, "pnpm-lock.yaml"))
        ).toBe(true);
      }
    );

    test("fails gracefully when lockfile is missing", async () => {
      const { root } = useFixture({ fixture: `../npm/basic` });
      const project = await MANAGERS.npm.read({
        workspaceRoot: root,
      });
      fs.rmSync(project.paths.lockfile);
      await pnpm.convertLock({
        project,
        logger: new Logger(),
        options: {
          interactive: false,
          dry: false,
        },
      });
      expect(existsSync(path.join(project.paths.root, "pnpm-lock.yaml"))).toBe(
        false
      );
    });
  });

  describe("clean", () => {
    test.each([
      [true, true],
      [false, true],
      [false, true],
      [true, false],
    ])("cleans %s pnpm workspaces", async (interactive, dry) => {
      const { root } = useFixture({ fixture: "basic" });
      const project = await pnpm.read({
        workspaceRoot: root,
      });

      await pnpm.clean({
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
