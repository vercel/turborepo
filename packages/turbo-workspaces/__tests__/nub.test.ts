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
    path.join(os.tmpdir(), "turbo-workspaces-nub-")
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

describe("nub", () => {
  describe("getWorkspacePackageManager", () => {
    it("reads nub from packageManager field", () => {
      const workspaceRoot = makeWorkspace({
        "package.json": { packageManager: "nub@0.1.0" }
      });

      expect(getWorkspacePackageManager({ workspaceRoot })).toEqual("nub");
    });

    it("reads nub from devEngines.packageManager", () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          devEngines: {
            packageManager: {
              name: "nub",
              version: "0.1.0"
            }
          }
        }
      });

      expect(getWorkspacePackageManager({ workspaceRoot })).toEqual("nub");
    });
  });

  describe("underlying lockfile resolution", () => {
    it("defaults to npm when no lockfile exists", () => {
      const workspaceRoot = makeWorkspace({
        "package.json": { packageManager: "nub@0.1.0" }
      });

      expect(getUnderlyingLockfileManager({ workspaceRoot })).toEqual("npm");
      expect(getUnderlyingLockfileName({ workspaceRoot })).toEqual(
        "package-lock.json"
      );
    });

    it("prefers bun over other lockfiles", () => {
      const workspaceRoot = makeWorkspace({
        "package.json": { packageManager: "nub@0.1.0" },
        "bun.lock": "",
        "pnpm-lock.yaml": "",
        "yarn.lock": "",
        "package-lock.json": "{}"
      });

      expect(getUnderlyingLockfileManager({ workspaceRoot })).toEqual("bun");
      expect(getUnderlyingLockfileName({ workspaceRoot })).toEqual("bun.lock");
    });

    it("detects pnpm when pnpm lockfile is present", () => {
      const workspaceRoot = makeWorkspace({
        "package.json": { packageManager: "nub@0.1.0" },
        "pnpm-lock.yaml": "lockfileVersion: '9.0'\n"
      });

      expect(getUnderlyingLockfileManager({ workspaceRoot })).toEqual("pnpm");
    });
  });

  describe("detection", () => {
    it("detects nub only via packageManager field", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {},
        "package-lock.json": "{}"
      });

      await expect(MANAGERS.nub.detect({ workspaceRoot })).resolves.toEqual(
        false
      );
      await expect(MANAGERS.npm.detect({ workspaceRoot })).resolves.toEqual(
        true
      );
    });

    it("detects nub when packageManager field is set", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": { packageManager: "nub@0.1.0" },
        "package-lock.json": "{}"
      });

      await expect(MANAGERS.nub.detect({ workspaceRoot })).resolves.toEqual(
        true
      );
    });
  });

  describe("getWorkspaceDetails", () => {
    it("returns nub as the package manager", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          name: "nub-project",
          packageManager: "nub@0.1.0",
          workspaces: ["packages/*"]
        },
        "package-lock.json": "{}"
      });

      const project = await getWorkspaceDetails({ root: workspaceRoot });

      expect(project.packageManager).toEqual("nub");
      expect(project.paths.lockfile).toEqual(
        path.join(workspaceRoot, "package-lock.json")
      );
      expect(project.workspaceData.globs).toEqual(["packages/*"]);
    });
  });

  describe("manager operations", () => {
    it("creates nub workspace metadata", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          name: "nub-project",
          packageManager: "npm@10.0.0",
          workspaces: ["packages/*"]
        },
        "package-lock.json": "{}"
      });
      const project = await MANAGERS.npm.read({ workspaceRoot });
      const packageJsonPath = path.join(workspaceRoot, "package.json");

      await MANAGERS.nub.create({
        project,
        to: { name: "nub", version: "0.1.0" },
        logger: new Logger({ dry: false, interactive: false })
      });

      const packageJson = fs.readJsonSync(packageJsonPath);
      expect(packageJson.devEngines?.packageManager).toEqual({
        name: "nub",
        version: "0.1.0"
      });
      expect(packageJson.packageManager).toBeUndefined();
    });

    it("removes nub workspace metadata", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          name: "nub-project",
          devEngines: {
            packageManager: {
              name: "nub",
              version: "0.1.0"
            }
          },
          workspaces: ["packages/*"]
        }
      });
      const project = await MANAGERS.nub.read({ workspaceRoot });

      await MANAGERS.nub.remove({
        project,
        to: { name: "npm", version: "10.0.0" },
        logger: new Logger({ dry: false, interactive: false })
      });

      const packageJson = fs.readJsonSync(
        path.join(workspaceRoot, "package.json")
      );
      expect(packageJson.workspaces).toBeUndefined();
      expect(packageJson.devEngines?.packageManager).toBeUndefined();
    });

    it("cleans the underlying lockfile", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          name: "nub-project",
          packageManager: "nub@0.1.0"
        },
        "package-lock.json": "{}"
      });
      const project = await MANAGERS.nub.read({ workspaceRoot });

      await MANAGERS.nub.clean({
        project,
        logger: new Logger({ dry: false, interactive: false })
      });

      expect(
        fs.existsSync(path.join(workspaceRoot, "package-lock.json"))
      ).toEqual(false);
    });

    it("removes foreign lockfiles when converting to nub", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          name: "pnpm-project",
          packageManager: "pnpm@9.0.0"
        },
        "pnpm-lock.yaml": "lockfileVersion: '9.0'\n"
      });
      const project = await MANAGERS.pnpm.read({ workspaceRoot });

      await MANAGERS.nub.convertLock({
        project,
        to: { name: "nub", version: "0.1.0" },
        logger: new Logger({ dry: false, interactive: false })
      });

      expect(fs.existsSync(path.join(workspaceRoot, "pnpm-lock.yaml"))).toEqual(
        false
      );
    });

    it("reads workspace data via the underlying lockfile manager", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          name: "nub-project",
          packageManager: "nub@0.1.0"
        },
        "pnpm-lock.yaml": "lockfileVersion: '9.0'\n",
        "pnpm-workspace.yaml": "packages:\n  - packages/*\n"
      });

      const project = await MANAGERS.nub.read({ workspaceRoot });

      expect(project.packageManager).toEqual("nub");
      expect(project.paths.lockfile).toEqual(
        path.join(workspaceRoot, "pnpm-lock.yaml")
      );
      expect(project.workspaceData.globs).toEqual(["packages/*"]);
    });

    it("throws when read is called on a non-nub project", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": { packageManager: "npm@10.0.0" },
        "package-lock.json": "{}"
      });

      await expect(MANAGERS.nub.read({ workspaceRoot })).rejects.toThrow(
        "Not a nub project"
      );
    });

    it("creates nub metadata without workspaces", async () => {
      const workspaceRoot = makeWorkspace({
        "package.json": {
          name: "nub-project",
          packageManager: "npm@10.0.0"
        },
        "package-lock.json": "{}"
      });
      const project = await MANAGERS.npm.read({ workspaceRoot });

      await MANAGERS.nub.create({
        project: { ...project, workspaceData: { globs: [], workspaces: [] } },
        to: { name: "nub", version: "0.1.0" },
        logger: new Logger({ dry: false, interactive: false })
      });

      const packageJson = fs.readJsonSync(
        path.join(workspaceRoot, "package.json")
      );
      expect(packageJson.devEngines?.packageManager).toEqual({
        name: "nub",
        version: "0.1.0"
      });
    });
  });
});
