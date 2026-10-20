import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  jest
} from "@jest/globals";
import { createProject } from "../src/create-project";
import * as examples from "../src/examples";

jest.mock("../src/examples", () => ({
  __esModule: true,
  downloadAndExtractExample: jest.fn(() => Promise.resolve()),
  downloadAndExtractRepo: jest.fn(() => Promise.resolve()),
  existsInRepo: jest.fn(() => Promise.resolve(true)),
  getRepoInfo: jest.fn(),
  hasRepo: jest.fn(() => Promise.resolve(true))
}));

describe("createProject", () => {
  let originalCwd: string;
  let baseDir: string;

  beforeEach(() => {
    originalCwd = process.cwd();
    baseDir = mkdtempSync(join(tmpdir(), "create-project-test-"));
    jest.clearAllMocks();
  });

  afterEach(() => {
    process.chdir(originalCwd);
    rmSync(baseDir, { recursive: true, force: true });
  });

  it("downloads the default example through the sparse example path", async () => {
    const appPath = join(baseDir, "my-app");

    await createProject({
      appPath,
      example: "basic",
      isDefaultExample: true
    });

    expect(examples.downloadAndExtractExample).toHaveBeenCalledWith(
      appPath,
      "basic"
    );
    expect(examples.downloadAndExtractRepo).not.toHaveBeenCalled();
  });
});
