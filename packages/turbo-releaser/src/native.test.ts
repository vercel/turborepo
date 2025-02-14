import { describe, it, mock } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import fs from "node:fs/promises";
import native from "./native";
import type { Platform } from "./types";

describe("generateNativePackage", () => {
  const outputDir = "/path/to/output";

  it("should generate package correctly for non-Windows platform", async (t) => {
    const mockRm = mock.fn((_path: string) => Promise.resolve());
    const mockMkdir = mock.fn((_path: string) => Promise.resolve());
    const mockCopyFile = mock.fn((_src: string, _dst: string) =>
      Promise.resolve()
    );
    const mockWriteFile = mock.fn((_path: string, _data: string) =>
      Promise.resolve()
    );

    t.mock.method(fs, "rm", mockRm);
    t.mock.method(fs, "mkdir", mockMkdir);
    t.mock.method(fs, "copyFile", mockCopyFile);
    t.mock.method(fs, "writeFile", mockWriteFile);

    const platform: Platform = { os: "darwin", arch: "x64" };
    const version = "1.0.0";
    await native.generateNativePackage({ platform, version, outputDir });

    // Assert rm was called correctly
    assert.equal(mockRm.mock.calls.length, 1);
    assert.equal(mockRm.mock.calls[0].arguments[0], outputDir);

    // Assert mkdir was called correctly
    assert.equal(mockMkdir.mock.calls.length, 1);
    assert.equal(
      mockMkdir.mock.calls[0].arguments[0],
      path.join(outputDir, "bin")
    );

    // Assert copyFile was called correctly
    assert.equal(mockCopyFile.mock.calls.length, 2);
    assert.ok(
      mockCopyFile.mock.calls[0].arguments[0].endsWith("template/README.md")
    );
    assert.equal(
      mockCopyFile.mock.calls[0].arguments[1],
      path.join(outputDir, "README.md")
    );
    assert.ok(
      mockCopyFile.mock.calls[1].arguments[0].endsWith("template/LICENSE")
    );
    assert.equal(
      mockCopyFile.mock.calls[1].arguments[1],
      path.join(outputDir, "LICENSE")
    );

    // Assert writeFile was called correctly
    assert.equal(mockWriteFile.mock.calls.length, 1);
    const [filePath, content] = mockWriteFile.mock.calls[0].arguments;
    assert.equal(filePath, path.join(outputDir, "package.json"));

    const packageJson = JSON.parse(content) as {
      name: string;
      version: string;
      description: string;
      os: Array<string>;
      cpu: Array<string>;
    };
    assert.equal(packageJson.name, `turbo-darwin-${native.archToHuman.x64}`);
    assert.equal(packageJson.version, version);
    assert.equal(
      packageJson.description,
      "The darwin-x64 binary for turbo, a monorepo build system."
    );
    assert.deepEqual(packageJson.os, ["darwin"]);
    assert.deepEqual(packageJson.cpu, ["x64"]);
  });

  it("should handle Windows platform correctly", async (t) => {
    const mockRm = mock.fn((_path: string) => Promise.resolve());
    const mockMkdir = mock.fn((_path: string) => Promise.resolve());
    const mockCopyFile = mock.fn((_src: string, _dst: string) =>
      Promise.resolve()
    );
    const mockWriteFile = mock.fn((_path: string, _data: string) =>
      Promise.resolve()
    );

    t.mock.method(fs, "rm", mockRm);
    t.mock.method(fs, "mkdir", mockMkdir);
    t.mock.method(fs, "copyFile", mockCopyFile);
    t.mock.method(fs, "writeFile", mockWriteFile);

    await native.generateNativePackage({
      platform: { os: "windows", arch: "x64" },
      version: "1.0.0",
      outputDir,
    });

    assert.equal(mockCopyFile.mock.calls.length, 3);
    assert.ok(
      mockCopyFile.mock.calls[0].arguments[0].endsWith("template/bin/turbo")
    );
    assert.equal(
      mockCopyFile.mock.calls[0].arguments[1],
      path.join(outputDir, "bin", "turbo")
    );
    const actualPackageJsonContents = mockWriteFile.mock.calls[0].arguments[1];
    const actualPackageJson = JSON.parse(actualPackageJsonContents) as {
      os: Array<string>;
    };
    assert.equal(actualPackageJson.os[0], "win32");
  });

  it("should propagate errors", async (t) => {
    const mockRm = mock.fn(() => {
      throw new Error("Failed to remove directory");
    });
    t.mock.method(fs, "rm", mockRm);

    await assert.rejects(
      native.generateNativePackage({
        platform: { os: "linux", arch: "x64" },
        version: "1.2.0",
        outputDir,
      }),
      { message: "Failed to remove directory" }
    );
  });
});

describe("archToHuman", () => {
  it("should map architectures correctly", () => {
    assert.equal(native.archToHuman.x64, "64");
    assert.equal(native.archToHuman.arm64, "arm64");
  });
});
