import os from "node:os";
import { describe, test, expect, beforeEach, jest } from "@jest/globals";
import execa from "execa";
import {
  getAvailablePackageManagers,
  getPackageManagersBinPaths,
} from "../src/managers";

// Mock dependencies
jest.mock("execa");
jest.mock("node:os");

const mockExeca = execa as jest.MockedFunction<typeof execa>;
const mockOs = os as jest.Mocked<typeof os>;

describe("managers", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockOs.tmpdir.mockReturnValue("/tmp");
  });

  describe("getAvailablePackageManagers", () => {
    test("should return all available package managers", async () => {
      mockExeca
        .mockResolvedValueOnce({ stdout: "1.22.19" } as any) // yarn
        .mockResolvedValueOnce({ stdout: "9.5.0" } as any) // npm
        .mockResolvedValueOnce({ stdout: "8.6.7" } as any) // pnpm
        .mockResolvedValueOnce({ stdout: "1.0.0" } as any); // bun

      const result = await getAvailablePackageManagers();

      expect(result).toEqual({
        yarn: "1.22.19",
        npm: "9.5.0",
        pnpm: "8.6.7",
        bun: "1.0.0",
      });
    });

    test("should return undefined for unavailable package managers", async () => {
      mockExeca
        .mockResolvedValueOnce({ stdout: "1.22.19" } as any) // yarn
        .mockRejectedValueOnce(new Error("npm not found")) // npm
        .mockResolvedValueOnce({ stdout: "8.6.7" } as any) // pnpm
        .mockRejectedValueOnce(new Error("bun not found")); // bun

      const result = await getAvailablePackageManagers();

      expect(result).toEqual({
        yarn: "1.22.19",
        npm: undefined,
        pnpm: "8.6.7",
        bun: undefined,
      });
    });

    describe("getPackageManagersBinPaths", () => {
      test("should return bin paths for all package managers", async () => {
        mockExeca
          .mockResolvedValueOnce({ stdout: "3.2.1" } as any) // yarn version (berry)
          .mockResolvedValueOnce({ stdout: "/usr/local/bin" } as any) // npm prefix
          .mockResolvedValueOnce({ stdout: "/usr/local/pnpm" } as any) // pnpm bin
          .mockResolvedValueOnce({ stdout: "/usr/local/bun" } as any); // bun bin

        const result = await getPackageManagersBinPaths();

        expect(result).toEqual({
          yarn: ".yarn/releases/yarn-3.2.1.cjs",
          npm: "/usr/local/bin",
          pnpm: "/usr/local/pnpm",
          bun: "/usr/local/bun",
        });
      });

      test("should handle yarn v1 global bin path", async () => {
        mockExeca
          .mockResolvedValueOnce({ stdout: "1.22.19" } as any) // yarn version check
          .mockResolvedValueOnce({ stdout: "/usr/local/bin" } as any) // npm prefix
          .mockResolvedValueOnce({ stdout: "/usr/local/pnpm" } as any) // pnpm bin
          .mockResolvedValueOnce({ stdout: "/usr/local/bun" } as any) // bun bin
          .mockResolvedValueOnce({ stdout: "/usr/local/yarn" } as any); // yarn global bin

        const result = await getPackageManagersBinPaths();

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

        const result = await getPackageManagersBinPaths();

        expect(result).toEqual({
          yarn: undefined,
          npm: undefined,
          pnpm: "/usr/local/pnpm",
          bun: undefined,
        });
      });

      test("should call execa with correct commands for bin paths", async () => {
        mockExeca.mockResolvedValue({ stdout: "1.0.0" } as any);

        await getPackageManagersBinPaths();

        // Verify yarn version check
        expect(mockExeca).toHaveBeenCalledWith("yarnpkg", ["--version"], {
          cwd: ".",
          env: { COREPACK_ENABLE_STRICT: "0" },
        });

        // Verify other package manager bin path commands
        expect(mockExeca).toHaveBeenCalledWith(
          "npm",
          ["config", "get", "prefix"],
          {
            cwd: "/tmp",
            env: { COREPACK_ENABLE_STRICT: "0" },
          }
        );
        expect(mockExeca).toHaveBeenCalledWith("pnpm", ["bin", "--global"], {
          cwd: "/tmp",
          env: { COREPACK_ENABLE_STRICT: "0" },
        });
        expect(mockExeca).toHaveBeenCalledWith("bun", ["pm", "--g", "bin"], {
          cwd: "/tmp",
          env: { COREPACK_ENABLE_STRICT: "0" },
        });
      });
    });
  });
});
