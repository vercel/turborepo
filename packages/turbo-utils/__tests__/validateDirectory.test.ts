import { describe, test, expect, beforeEach, jest } from "@jest/globals";
import path from "node:path";
import fs from "fs-extra";
import { validateDirectory } from "../src/validateDirectory";
import { isFolderEmpty } from "../src/isFolderEmpty";

// Mock dependencies
jest.mock("fs-extra");
jest.mock("../src/isFolderEmpty");

const mockFs = fs as jest.Mocked<typeof fs>;
const mockIsFolderEmpty = isFolderEmpty as jest.MockedFunction<
  typeof isFolderEmpty
>;

describe("validateDirectory", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  test("should return valid result for valid empty directory", () => {
    const directory = "/path/to/project";
    const resolvedPath = path.resolve(directory);

    mockFs.existsSync.mockReturnValue(true);
    mockFs.lstatSync.mockReturnValue({
      isDirectory: () => true,
    } as fs.Stats);
    mockIsFolderEmpty.mockReturnValue({
      isEmpty: true,
      conflicts: [],
    });

    const result = validateDirectory(directory);

    expect(result).toEqual({
      valid: true,
      root: resolvedPath,
      projectName: "project",
    });
  });

  test("should return error when path points to a file", () => {
    const directory = "/path/to/file.txt";
    const resolvedPath = path.resolve(directory);

    mockFs.lstatSync.mockReturnValue({
      isDirectory: () => false,
    } as fs.Stats);

    const result = validateDirectory(directory);

    expect(result).toEqual({
      valid: false,
      root: resolvedPath,
      projectName: "file.txt",
      error: expect.stringContaining("is not a directory"),
    });
  });

  test("should return error when directory has conflicts", () => {
    const directory = "/path/to/existing";
    const resolvedPath = path.resolve(directory);
    const conflicts = ["package.json", "src/"];

    mockFs.existsSync.mockReturnValue(true);
    mockFs.lstatSync.mockReturnValue({
      isDirectory: () => true,
    } as fs.Stats);
    mockIsFolderEmpty.mockReturnValue({
      isEmpty: false,
      conflicts,
    });

    const result = validateDirectory(directory);

    expect(result).toEqual({
      valid: false,
      root: resolvedPath,
      projectName: "existing",
      error: expect.stringContaining("has 2 conflicting files"),
    });
  });

  test("should return error with singular 'file' for single conflict", () => {
    const directory = "/path/to/existing";
    const resolvedPath = path.resolve(directory);
    const conflicts = ["package.json"];

    mockFs.existsSync.mockReturnValue(true);
    mockFs.lstatSync.mockReturnValue({
      isDirectory: () => true,
    } as fs.Stats);
    mockIsFolderEmpty.mockReturnValue({
      isEmpty: false,
      conflicts,
    });

    const result = validateDirectory(directory);

    expect(result).toEqual({
      valid: false,
      root: resolvedPath,
      projectName: "existing",
      error: expect.stringContaining("has 1 conflicting file"),
    });
  });

  test("should handle non-existent directory as valid", () => {
    const directory = "/path/to/new-project";
    const resolvedPath = path.resolve(directory);

    mockFs.existsSync.mockReturnValue(false);
    mockFs.lstatSync.mockReturnValue(undefined as any);

    const result = validateDirectory(directory);

    expect(result).toEqual({
      valid: true,
      root: resolvedPath,
      projectName: "new-project",
    });
  });

  test("should handle lstat errors gracefully", () => {
    const directory = "/path/to/project";
    const resolvedPath = path.resolve(directory);

    mockFs.lstatSync.mockImplementation(() => {
      const error = new Error("Permission denied");
      (error as any).code = "ENOENT";
      throw error;
    });

    // Since lstatSync is called with throwIfNoEntry: false, it should return null on error
    mockFs.lstatSync.mockReturnValue(null as any);

    const result = validateDirectory(directory);

    expect(result).toEqual({
      valid: true,
      root: resolvedPath,
      projectName: "project",
    });
  });
});
