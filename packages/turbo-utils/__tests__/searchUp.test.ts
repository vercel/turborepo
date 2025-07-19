import { describe, test, expect, beforeEach, jest } from "@jest/globals";
import fs from "node:fs";
import path from "node:path";
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

  test("should use content check when provided", () => {
    const targetFile = "config.json";
    const validContent = '{"valid": true}';
    const invalidContent = '{"valid": false}';

    mockFs.readFileSync
      .mockReturnValueOnce(Buffer.from(invalidContent)) // first call returns invalid
      .mockReturnValueOnce(Buffer.from(validContent)); // second call returns valid

    const contentCheck = (content: string) => {
      const parsed = JSON.parse(content);
      return parsed.valid === true;
    };

    // Mock existsSync to always return true, but content check will determine validity
    mockFs.readFileSync.mockImplementation((filePath: any) => {
      if (filePath.toString().includes("/path/to/project/src/components")) {
        return invalidContent as any;
      }
      if (filePath.toString().includes("/path/to/project/src")) {
        return validContent as any;
      }
      throw new Error("File not found");
    });

    const result = searchUp({
      target: targetFile,
      cwd: mockCwd,
      contentCheck,
    });

    expect(result).toBe("/path/to/project/src");
  });

  test("should handle read errors gracefully during content check", () => {
    const targetFile = "config.json";

    mockFs.readFileSync.mockImplementation(() => {
      throw new Error("Permission denied");
    });

    const contentCheck = () => true;

    const result = searchUp({
      target: targetFile,
      cwd: mockCwd,
      contentCheck,
    });

    expect(result).toBeNull();
  });

  test("should stop at root directory", () => {
    mockFs.existsSync.mockReturnValue(false);

    const result = searchUp({ target: "package.json", cwd: "/path/very/deep" });

    expect(result).toBeNull();
    expect(mockFs.existsSync).toHaveBeenCalled();
  });
});
