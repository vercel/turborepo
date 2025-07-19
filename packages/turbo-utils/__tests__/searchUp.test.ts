import fs from "node:fs";
import path from "node:path";
import { describe, test, expect, beforeEach, jest } from "@jest/globals";
import { searchUp } from "../src/searchUp";

// Mock fs module
jest.mock("node:fs");
const mockFs = fs as jest.Mocked<typeof fs>;

describe("searchUp", () => {
  const mockCwd = "/path/to/project/src/components";
  const mockRoot = process.platform === "win32" ? "C:\\" : "/";

  beforeEach(() => {
    jest.clearAllMocks();
    // Mock path.parse to return consistent root
    jest.spyOn(path, "parse").mockImplementation((pathString: string) => ({
      root: mockRoot,
      dir: path.dirname(pathString),
      base: path.basename(pathString),
      ext: path.extname(pathString),
      name: path.basename(pathString, path.extname(pathString)),
    }));
  });

  test("should find file in current directory", () => {
    const targetFile = "package.json";
    mockFs.existsSync.mockImplementation(
      (filePath: any) => filePath === path.join(mockCwd, targetFile)
    );

    const result = searchUp({ target: targetFile, cwd: mockCwd });

    expect(result).toBe(mockCwd);
    expect(mockFs.existsSync).toHaveBeenCalledWith(
      path.join(mockCwd, targetFile)
    );
  });

  test("should find file in parent directory", () => {
    const targetFile = "turbo.json";
    const parentDir = "/path/to/project";

    mockFs.existsSync.mockImplementation(
      (filePath: any) => filePath === path.join(parentDir, targetFile)
    );

    const result = searchUp({ target: targetFile, cwd: mockCwd });

    expect(result).toBe(parentDir);
  });

  test("should return null when file not found", () => {
    mockFs.existsSync.mockReturnValue(false);

    const result = searchUp({ target: "nonexistent.json", cwd: mockCwd });

    expect(result).toBeNull();
  });
});
