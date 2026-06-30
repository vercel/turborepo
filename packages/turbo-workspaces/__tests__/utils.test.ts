import { describe, it, expect } from "@jest/globals";
import os from "node:os";
import path from "node:path";
import fs from "fs-extra";
import type { Project } from "../src/types";
import {
  getWorkspacePackageManager,
  isCompatibleWithBunWorkspaces
} from "../src/utils";

function makeWorkspace(packageJson: unknown): string {
  const workspaceRoot = fs.mkdtempSync(
    path.join(os.tmpdir(), "turbo-workspaces-")
  );
  fs.writeJsonSync(path.join(workspaceRoot, "package.json"), packageJson);
  return workspaceRoot;
}

describe("utils", () => {
  describe("isCompatibleWithBunWorkspace", () => {
    it.each([
      { globs: ["apps/*"], expected: true },
      { globs: ["apps/*", "packages/*"], expected: true },
      { globs: ["*"], expected: true },
      { globs: ["workspaces/**/*"], expected: false },
      { globs: ["apps/*", "packages/**/*"], expected: false },
      { globs: ["apps/*", "packages/*/utils/*"], expected: false },
      { globs: ["internal-*/*"], expected: false }
    ])("should return $result when given %globs", ({ globs, expected }) => {
      const result = isCompatibleWithBunWorkspaces({
        project: {
          workspaceData: { globs }
        } as Project
      });
      expect(result).toEqual(expected);
    });
  });

  describe("getWorkspacePackageManager", () => {
    it.each([
      ["npm", "10.5.0"],
      ["pnpm", "9.12.3"],
      ["yarn", "4.5.0+sha224.abc"],
      ["bun", "1.1.0"],
      ["nub", "0.1.0"]
    ] as const)("reads %s from devEngines.packageManager", (name, version) => {
      const workspaceRoot = makeWorkspace({
        devEngines: {
          packageManager: {
            name,
            version,
            onFail: "warn",
            ignored: true
          }
        }
      });

      expect(getWorkspacePackageManager({ workspaceRoot })).toEqual(name);
    });

    it("prefers top-level packageManager over devEngines.packageManager", () => {
      const workspaceRoot = makeWorkspace({
        packageManager: "pnpm@9.12.3",
        devEngines: {
          packageManager: {
            name: "npm",
            version: "10.5.0"
          }
        }
      });

      expect(getWorkspacePackageManager({ workspaceRoot })).toEqual("pnpm");
    });

    it("ignores malformed devEngines.packageManager when top-level packageManager is present", () => {
      const workspaceRoot = makeWorkspace({
        packageManager: "npm@10.5.0",
        devEngines: {
          packageManager: []
        }
      });

      expect(getWorkspacePackageManager({ workspaceRoot })).toEqual("npm");
    });

    it("returns undefined when no package manager declaration exists", () => {
      const workspaceRoot = makeWorkspace({});

      expect(getWorkspacePackageManager({ workspaceRoot })).toBeUndefined();
    });

    it.each([
      [{ devEngines: [] }, "`devEngines` must be an object"],
      [{ devEngines: null }, "`devEngines` must be an object"],
      [
        { devEngines: { packageManager: [] } },
        "`devEngines.packageManager` must be an object"
      ],
      [
        { devEngines: { packageManager: null } },
        "`devEngines.packageManager` must be an object"
      ],
      [{ devEngines: { packageManager: {} } }, "expected"],
      [
        { devEngines: { packageManager: { version: "9.12.3" } } },
        "name` is required"
      ],
      [
        { devEngines: { packageManager: { name: 1, version: "9.12.3" } } },
        "name` must be a string"
      ],
      [
        { devEngines: { packageManager: { name: "", version: "9.12.3" } } },
        "name` must not be empty"
      ],
      [
        {
          devEngines: { packageManager: { name: " pnpm", version: "9.12.3" } }
        },
        "name` must not contain"
      ],
      [
        { devEngines: { packageManager: { name: "pip", version: 1 } } },
        "name` must be one of"
      ],
      [
        { devEngines: { packageManager: { name: "pnpm" } } },
        "version` is required"
      ],
      [
        { devEngines: { packageManager: { name: "pnpm", version: 1 } } },
        "version` must be a string"
      ],
      [
        { devEngines: { packageManager: { name: "pnpm", version: "" } } },
        "version` must not be empty"
      ],
      [
        {
          devEngines: { packageManager: { name: "pnpm", version: " 9.12.3" } }
        },
        "version` must not contain"
      ],
      [
        { devEngines: { packageManager: { name: "pnpm", version: "^9.0.0" } } },
        "exact semantic version"
      ],
      [
        {
          devEngines: {
            packageManager: {
              name: "pnpm",
              version: "https://registry.npmjs.org/pnpm/-/pnpm-9.12.3.tgz"
            }
          }
        },
        "exact semantic version"
      ],
      [
        { devEngines: { packageManager: { name: "pnpm", version: "9" } } },
        "exact semantic version"
      ],
      [
        {
          devEngines: {
            packageManager: {
              name: "pnpm",
              version: "9.12.3+sha512.Purxi/Zex=="
            }
          }
        },
        "exact semantic version"
      ]
    ])(
      "rejects invalid devEngines.packageManager %#",
      (packageJson, message) => {
        const workspaceRoot = makeWorkspace(packageJson);

        expect(() => getWorkspacePackageManager({ workspaceRoot })).toThrow(
          message
        );
      }
    );
  });
});
