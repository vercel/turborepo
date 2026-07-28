import os from "node:os";
import path from "node:path";
import fs from "fs-extra";
import { describe, it, expect } from "@jest/globals";
import { Logger } from "../src/logger";
import { getWorkspaceDetails } from "../src/get-workspace-details";
import {
  getUnderlyingLockfileManager,
  getUnderlyingLockfileName,
  getWorkspacePackageManager
} from "../src/utils";
import { MANAGERS } from "../src/managers";

function makeWorkspace(files: Record<string, unknown>): string {
  const workspaceRoot = fs.mkdtempSync(
    path.join(os.tmpdir(), "turbo-workspaces-aube-")
  );

  for (const [filePath, content] of Object.entries(files)) {
    const absolutePath = path.join(workspaceRoot, filePath);
    fs.mkdirSync(path.dirname(absolutePath), { recursive: true });
    if (typeof content === "string") {
      fs.writeFileSync(absolutePath, content);
    } else {
      fs.writeJsonSync(absolutePath, content);
    }
  }

  return workspaceRoot;
}

describe("aube", () => {
  describe("getWorkspacePackageManager", () => {
    it("reads aube from packageManager field", () => {
      const workspaceRoot = makeWorkspace({
        "package.json": { packageManager: "aube@0.1.0" }
      });

      expect(getWorkspacePackageManager({ workspaceRoot })).toEqual("aube");
    });

    it("reads aube from devEngines.packageManager", () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          devEngines: {
            packageManager: {
              name: "aube",
              version: "0.1.0"
            }
          }
        }
      });

      expect(getWorkspacePackageManager({ workspaceRoot })).toEqual("aube");
    });
  });

  describe("underlying lockfile resolution", () => {
    it("defaults to npm when no lockfile exists", () => {
      const workspaceRoot = makeWorkspace({
        "package.json": { packageManager: "aube@0.1.0" }
      });

      expect(getUnderlyingLockfileManager({ workspaceRoot })).toEqual("npm");
      expect(getUnderlyingLockfileName({ workspaceRoot })).toEqual(
        "package-lock.json"
      );
    });

    it("uses aube-lock.yaml when present", () => {
      const workspaceRoot = makeWorkspace({
        "package.json": { packageManager: "aube@0.1.0" },
        "aube-lock.yaml": "lockfileVersion: '9.0'\n"
      });

      expect(getUnderlyingLockfileManager({ workspaceRoot })).toEqual("pnpm");
      expect(getUnderlyingLockfileName({ workspaceRoot })).toEqual(
        "aube-lock.yaml"
      );
    });

    it("prefers bun over non-native foreign lockfiles", () => {
      const workspaceRoot = makeWorkspace({
        "package.json": { packageManager: "aube@0.1.0" },
        "bun.lock": "",
        "pnpm-lock.yaml": "",
        "yarn.lock": "",
        "package-lock.json": "{}"
      });

      expect(getUnderlyingLockfileManager({ workspaceRoot })).toEqual("bun");
      expect(getUnderlyingLockfileName({ workspaceRoot })).toEqual("bun.lock");
    });
  });

  describe("detection", () => {
    it("detects aube only via packageManager field", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {},
        "aube-lock.yaml": "lockfileVersion: '9.0'\n"
      });

      await expect(MANAGERS.aube.detect({ workspaceRoot })).resolves.toEqual(
        false
      );
    });

    it("detects aube when packageManager field is set", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": { packageManager: "aube@0.1.0" },
        "aube-lock.yaml": "lockfileVersion: '9.0'\n"
      });

      await expect(MANAGERS.aube.detect({ workspaceRoot })).resolves.toEqual(
        true
      );
    });
  });

  describe("getWorkspaceDetails", () => {
    it("returns aube as the package manager", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          name: "aube-project",
          packageManager: "aube@0.1.0",
          workspaces: ["packages/*"]
        },
        "package-lock.json": "{}"
      });

      const project = await getWorkspaceDetails({ root: workspaceRoot });

      expect(project.packageManager).toEqual("aube");
      expect(project.paths.lockfile).toEqual(
        path.join(workspaceRoot, "package-lock.json")
      );
      expect(project.workspaceData.globs).toEqual(["packages/*"]);
    });
  });

  describe("manager operations", () => {
    it("creates aube workspace metadata", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          name: "aube-project",
          packageManager: "npm@10.0.0",
          workspaces: ["packages/*"]
        },
        "package-lock.json": "{}"
      });
      const project = await MANAGERS.npm.read({ workspaceRoot });
      const packageJsonPath = path.join(workspaceRoot, "package.json");

      await MANAGERS.aube.create({
        project,
        to: { name: "aube", version: "0.1.0" },
        logger: new Logger({ dry: false, interactive: false })
      });

      const packageJson = fs.readJsonSync(packageJsonPath);
      expect(packageJson.devEngines?.packageManager).toEqual({
        name: "aube",
        version: "0.1.0"
      });
      expect(packageJson.packageManager).toBeUndefined();
    });

    it("creates aube metadata without workspaces", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          name: "aube-project",
          packageManager: "npm@10.0.0"
        },
        "package-lock.json": "{}"
      });
      const project = await MANAGERS.npm.read({ workspaceRoot });

      await MANAGERS.aube.create({
        project: { ...project, workspaceData: { globs: [], workspaces: [] } },
        to: { name: "aube", version: "0.1.0" },
        logger: new Logger({ dry: false, interactive: false })
      });

      const packageJson = fs.readJsonSync(
        path.join(workspaceRoot, "package.json")
      );
      expect(packageJson.devEngines?.packageManager).toEqual({
        name: "aube",
        version: "0.1.0"
      });
    });

    it("does not write aube metadata during dry creation", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          name: "aube-project",
          packageManager: "npm@10.0.0"
        },
        "package-lock.json": "{}"
      });
      const project = await MANAGERS.npm.read({ workspaceRoot });

      await MANAGERS.aube.create({
        project: { ...project, workspaceData: { globs: [], workspaces: [] } },
        to: { name: "aube", version: "0.1.0" },
        logger: new Logger({ dry: true, interactive: false }),
        options: { dry: true }
      });

      const packageJson = fs.readJsonSync(
        path.join(workspaceRoot, "package.json")
      );
      expect(packageJson.devEngines?.packageManager).toBeUndefined();
    });

    it("removes aube workspace metadata", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          name: "aube-project",
          devEngines: {
            packageManager: {
              name: "aube",
              version: "0.1.0"
            }
          },
          workspaces: ["packages/*"]
        },
        "packages/app/package.json": { name: "app" },
        "packages/app/node_modules/.keep": ""
      });
      const project = await MANAGERS.aube.read({ workspaceRoot });

      await MANAGERS.aube.remove({
        project,
        to: { name: "npm", version: "10.0.0" },
        logger: new Logger({ dry: false, interactive: false })
      });

      const packageJson = fs.readJsonSync(
        path.join(workspaceRoot, "package.json")
      );
      expect(packageJson.workspaces).toBeUndefined();
      expect(packageJson.devEngines?.packageManager).toBeUndefined();
      expect(
        fs.existsSync(path.join(workspaceRoot, "packages/app/node_modules"))
      ).toEqual(false);
    });

    it("does not remove aube metadata during dry removal", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          name: "aube-project",
          devEngines: {
            packageManager: {
              name: "aube",
              version: "0.1.0"
            }
          },
          workspaces: ["packages/*"]
        }
      });
      const project = await MANAGERS.aube.read({ workspaceRoot });

      await MANAGERS.aube.remove({
        project,
        to: { name: "npm", version: "10.0.0" },
        logger: new Logger({ dry: true, interactive: false }),
        options: { dry: true }
      });

      const packageJson = fs.readJsonSync(
        path.join(workspaceRoot, "package.json")
      );
      expect(packageJson.workspaces).toEqual(["packages/*"]);
      expect(packageJson.devEngines?.packageManager).toEqual({
        name: "aube",
        version: "0.1.0"
      });
    });

    it("cleans the underlying lockfile", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          name: "aube-project",
          packageManager: "aube@0.1.0"
        },
        "aube-lock.yaml": "lockfileVersion: '9.0'\n"
      });
      const project = await MANAGERS.aube.read({ workspaceRoot });

      await MANAGERS.aube.clean({
        project,
        logger: new Logger({ dry: false, interactive: false })
      });

      expect(fs.existsSync(path.join(workspaceRoot, "aube-lock.yaml"))).toEqual(
        false
      );
    });

    it("does not clean the underlying lockfile during dry clean", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          name: "aube-project",
          packageManager: "aube@0.1.0"
        },
        "aube-lock.yaml": "lockfileVersion: '9.0'\n"
      });
      const project = await MANAGERS.aube.read({ workspaceRoot });

      await MANAGERS.aube.clean({
        project,
        logger: new Logger({ dry: true, interactive: false }),
        options: { dry: true }
      });

      expect(fs.existsSync(path.join(workspaceRoot, "aube-lock.yaml"))).toEqual(
        true
      );
    });

    it("removes foreign lockfiles when converting to aube", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          name: "pnpm-project",
          packageManager: "pnpm@9.0.0"
        },
        "pnpm-lock.yaml": "lockfileVersion: '9.0'\n"
      });
      const project = await MANAGERS.pnpm.read({ workspaceRoot });

      await MANAGERS.aube.convertLock({
        project,
        to: { name: "aube", version: "0.1.0" },
        logger: new Logger({ dry: false, interactive: false })
      });

      expect(fs.existsSync(path.join(workspaceRoot, "pnpm-lock.yaml"))).toEqual(
        false
      );
    });

    it("keeps compatible lockfiles when converting to aube", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          name: "npm-project",
          packageManager: "npm@10.0.0"
        },
        "package-lock.json": "{}"
      });
      const project = await MANAGERS.npm.read({ workspaceRoot });

      await MANAGERS.aube.convertLock({
        project,
        to: { name: "aube", version: "0.1.0" },
        logger: new Logger({ dry: false, interactive: false })
      });

      expect(
        fs.existsSync(path.join(workspaceRoot, "package-lock.json"))
      ).toEqual(true);
    });

    it("reads workspace data from aube-workspace.yaml", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          name: "aube-project",
          packageManager: "aube@0.1.0"
        },
        "aube-lock.yaml": "lockfileVersion: '9.0'\n",
        "aube-workspace.yaml": "packages:\n  - packages/*\n"
      });

      const project = await MANAGERS.aube.read({ workspaceRoot });

      expect(project.packageManager).toEqual("aube");
      expect(project.paths.lockfile).toEqual(
        path.join(workspaceRoot, "aube-lock.yaml")
      );
      expect(project.paths.workspaceConfig).toEqual(
        path.join(workspaceRoot, "aube-workspace.yaml")
      );
      expect(project.workspaceData.globs).toEqual(["packages/*"]);
    });

    it("throws when read is called on a non-aube project", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": { packageManager: "npm@10.0.0" },
        "package-lock.json": "{}"
      });

      await expect(MANAGERS.aube.read({ workspaceRoot })).rejects.toThrow(
        "Not an aube project"
      );
    });
  });
});
