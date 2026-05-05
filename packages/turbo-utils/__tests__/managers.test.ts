import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  describe,
  test,
  expect,
  beforeEach,
  afterEach,
  jest
} from "@jest/globals";
import execa from "execa";
import {
  getAvailablePackageManagers,
  getPackageManagersBinPaths
} from "../src/managers";

// Mock dependencies
jest.mock("execa");
jest.mock("node:os");

const mockExeca = jest.mocked(execa);
const mockOs = os as jest.Mocked<typeof os>;
const realOs = jest.requireActual<typeof import("node:os")>("node:os");
const MISSING_PROJECT_ROOT = path.join(
  realOs.tmpdir(),
  "turbo-managers-missing"
);

const tempDirs: Array<string> = [];

function createProject(files: Record<string, string>) {
  const projectRoot = fs.mkdtempSync(
    path.join(realOs.tmpdir(), "turbo-managers-")
  );
  tempDirs.push(projectRoot);

  for (const [filePath, content] of Object.entries(files)) {
    const absolutePath = path.join(projectRoot, filePath);
    fs.mkdirSync(path.dirname(absolutePath), { recursive: true });
    fs.writeFileSync(absolutePath, content);
  }

  return projectRoot;
}

describe("managers", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockOs.tmpdir.mockReturnValue("/tmp");
  });

  afterEach(() => {
    for (const tempDir of tempDirs.splice(0)) {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });

  describe("getAvailablePackageManagers", () => {
    test("should return all available package managers", async () => {
      mockExeca
        .mockResolvedValueOnce({ stdout: "1.22.19" } as any) // yarn
        .mockResolvedValueOnce({ stdout: "9.5.0" } as any) // npm
        .mockResolvedValueOnce({ stdout: "8.6.7" } as any) // pnpm
        .mockResolvedValueOnce({ stdout: "1.0.0" } as any); // bun

      const result = await getAvailablePackageManagers({
        projectRoot: MISSING_PROJECT_ROOT
      });

      expect(result).toEqual({
        yarn: "1.22.19",
        npm: "9.5.0",
        pnpm: "8.6.7",
        bun: "1.0.0"
      });
    });

    test("should return undefined for unavailable package managers", async () => {
      mockExeca
        .mockResolvedValueOnce({ stdout: "1.22.19" } as any) // yarn
        .mockRejectedValueOnce(new Error("npm not found")) // npm
        .mockResolvedValueOnce({ stdout: "8.6.7" } as any) // pnpm
        .mockRejectedValueOnce(new Error("bun not found")); // bun

      const result = await getAvailablePackageManagers({
        projectRoot: MISSING_PROJECT_ROOT
      });

      expect(result).toEqual({
        yarn: "1.22.19",
        npm: undefined,
        pnpm: "8.6.7",
        bun: undefined
      });
    });

    test("should infer project yarn version from packageManager", async () => {
      const projectRoot = createProject({
        "package.json": JSON.stringify({ packageManager: "yarn@4.5.1" })
      });
      mockExeca
        .mockResolvedValueOnce({ stdout: "9.5.0" } as any) // npm
        .mockResolvedValueOnce({ stdout: "8.6.7" } as any) // pnpm
        .mockResolvedValueOnce({ stdout: "1.0.0" } as any); // bun

      const result = await getAvailablePackageManagers({ projectRoot });

      expect(result).toEqual({
        yarn: "4.5.1",
        npm: "9.5.0",
        pnpm: "8.6.7",
        bun: "1.0.0"
      });
      expect(mockExeca.mock.calls.map(([command]) => command)).toEqual([
        "npm",
        "pnpm",
        "bun"
      ]);
    });

    test("should infer project yarn version from conventional yarnPath", async () => {
      const projectRoot = createProject({
        ".yarnrc.yml": "yarnPath: .yarn/releases/yarn-3.2.1.cjs\n"
      });
      mockExeca
        .mockResolvedValueOnce({ stdout: "9.5.0" } as any) // npm
        .mockResolvedValueOnce({ stdout: "8.6.7" } as any) // pnpm
        .mockResolvedValueOnce({ stdout: "1.0.0" } as any); // bun

      const result = await getAvailablePackageManagers({ projectRoot });

      expect(result.yarn).toBe("3.2.1");
      expect(mockExeca.mock.calls.map(([command]) => command)).toEqual([
        "npm",
        "pnpm",
        "bun"
      ]);
    });

    test("should not execute or fall back when yarnPath is custom", async () => {
      const projectRoot = createProject({
        ".yarnrc.yml": "yarnPath: ./scripts/yarn.cjs\n"
      });
      mockExeca
        .mockResolvedValueOnce({ stdout: "9.5.0" } as any) // npm
        .mockResolvedValueOnce({ stdout: "8.6.7" } as any) // pnpm
        .mockResolvedValueOnce({ stdout: "1.0.0" } as any); // bun

      const result = await getAvailablePackageManagers({ projectRoot });

      expect(result.yarn).toBeUndefined();
      expect(mockExeca.mock.calls.map(([command]) => command)).toEqual([
        "npm",
        "pnpm",
        "bun"
      ]);
    });
  });

  describe("getPackageManagersBinPaths", () => {
    test("should return bin paths for all package managers", async () => {
      mockExeca
        .mockResolvedValueOnce({ stdout: "3.2.1" } as any) // yarn version (berry)
        .mockResolvedValueOnce({ stdout: "/usr/local/bin" } as any) // npm prefix
        .mockResolvedValueOnce({ stdout: "/usr/local/pnpm" } as any) // pnpm bin
        .mockResolvedValueOnce({ stdout: "/usr/local/bun" } as any); // bun bin

      const result = await getPackageManagersBinPaths({
        projectRoot: MISSING_PROJECT_ROOT
      });

      expect(result).toEqual({
        yarn: ".yarn/releases/yarn-3.2.1.cjs",
        npm: "/usr/local/bin",
        pnpm: "/usr/local/pnpm",
        bun: "/usr/local/bun"
      });
    });

    test("should handle yarn v1 global bin path", async () => {
      mockExeca
        .mockResolvedValueOnce({ stdout: "1.22.19" } as any) // yarn version check
        .mockResolvedValueOnce({ stdout: "/usr/local/bin" } as any) // npm prefix
        .mockResolvedValueOnce({ stdout: "/usr/local/pnpm" } as any) // pnpm bin
        .mockResolvedValueOnce({ stdout: "/usr/local/bun" } as any) // bun bin
        .mockResolvedValueOnce({ stdout: "/usr/local/yarn" } as any); // yarn global bin

      const result = await getPackageManagersBinPaths({
        projectRoot: MISSING_PROJECT_ROOT
      });

      expect(result.yarn).toBe("/usr/local/yarn");
      expect(result.npm).toBe("/usr/local/bin");
      expect(result.pnpm).toBe("/usr/local/pnpm");
      expect(result.bun).toBe("/usr/local/bun");
    });

    test("should return undefined for failed package manager checks", async () => {
      mockExeca
        .mockRejectedValueOnce(new Error("yarn not found")) // yarn
        .mockRejectedValueOnce(new Error("npm not found")) // npm
        .mockResolvedValueOnce({ stdout: "/usr/local/pnpm" } as any) // pnpm
        .mockRejectedValueOnce(new Error("bun not found")); // bun

      const result = await getPackageManagersBinPaths({
        projectRoot: MISSING_PROJECT_ROOT
      });

      expect(result).toEqual({
        yarn: undefined,
        npm: undefined,
        pnpm: "/usr/local/pnpm",
        bun: undefined
      });
    });

    test("should call execa with correct commands for bin paths", async () => {
      mockExeca.mockResolvedValue({ stdout: "1.0.0" } as any);

      await getPackageManagersBinPaths({ projectRoot: MISSING_PROJECT_ROOT });

      expect(mockExeca).toHaveBeenCalledWith("yarnpkg", ["--version"], {
        cwd: "/tmp",
        env: { COREPACK_ENABLE_STRICT: "0" },
        timeout: 5000
      });

      expect(mockExeca).toHaveBeenCalledWith(
        "npm",
        ["config", "get", "prefix"],
        {
          cwd: "/tmp",
          env: { COREPACK_ENABLE_STRICT: "0" },
          timeout: 5000
        }
      );
      expect(mockExeca).toHaveBeenCalledWith("pnpm", ["bin", "--global"], {
        cwd: "/tmp",
        env: { COREPACK_ENABLE_STRICT: "0" },
        timeout: 5000
      });
      expect(mockExeca).toHaveBeenCalledWith("bun", ["pm", "--g", "bin"], {
        cwd: "/tmp",
        env: { COREPACK_ENABLE_STRICT: "0" },
        timeout: 5000
      });
    });

    test("should infer yarn berry bin path without executing yarn", async () => {
      const projectRoot = createProject({
        "package.json": JSON.stringify({ packageManager: "yarn@4.5.1" })
      });
      mockExeca
        .mockResolvedValueOnce({ stdout: "/usr/local/bin" } as any) // npm prefix
        .mockResolvedValueOnce({ stdout: "/usr/local/pnpm" } as any) // pnpm bin
        .mockResolvedValueOnce({ stdout: "/usr/local/bun" } as any); // bun bin

      const result = await getPackageManagersBinPaths({ projectRoot });

      expect(result).toEqual({
        yarn: ".yarn/releases/yarn-4.5.1.cjs",
        npm: "/usr/local/bin",
        pnpm: "/usr/local/pnpm",
        bun: "/usr/local/bun"
      });
      expect(mockExeca.mock.calls.map(([command]) => command)).toEqual([
        "npm",
        "pnpm",
        "bun"
      ]);
    });
  });
});
