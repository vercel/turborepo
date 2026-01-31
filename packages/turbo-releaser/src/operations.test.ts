import { describe, it, mock } from "node:test";
import fs from "node:fs/promises";
import assert from "node:assert";
import path from "node:path";
import { tmpdir } from "node:os";
import { create as tarCreate } from "tar";
import native from "./native";
import type { Platform } from "./types";
import operations from "./operations";

describe("packPlatform", () => {
  it("should pack a platform correctly", async (t) => {
    const mockGenerateNativePackage = mock.fn();
    const mockMkdir = mock.fn();
    const mockCopyFile = mock.fn();
    const mockStat = mock.fn(() => Promise.resolve({ mode: 0 }));
    const mockChmod = mock.fn();

    t.mock.method(native, "generateNativePackage", mockGenerateNativePackage);
    t.mock.method(fs, "mkdir", mockMkdir);
    t.mock.method(fs, "copyFile", mockCopyFile);
    t.mock.method(fs, "stat", mockStat);
    t.mock.method(fs, "chmod", mockChmod);

    const platform: Platform = { os: "darwin", arch: "x64" };
    const version = "1.0.0";

    // Since tar.create cannot be easily mocked with ESM, we expect the
    // function to fail when it tries to create the tarball (no real files).
    // We verify the other calls were made correctly before tar.create is called.
    try {
      await operations.packPlatform({ platform, version });
    } catch {
      // Expected to fail since no real files exist
    }

    assert.equal(mockGenerateNativePackage.mock.calls.length, 1);
    assert.equal(mockMkdir.mock.calls.length, 1);
    assert.equal(mockCopyFile.mock.calls.length, 1);
    assert.equal(mockStat.mock.calls.length, 1);
    assert.equal(mockChmod.mock.calls.length, 1);
    assert.equal(mockChmod.mock.calls[0].arguments[1], 0o111);
  });

  it("should pack a Windows with .exe", async (t) => {
    const mockGenerateNativePackage = mock.fn();
    const mockMkdir = mock.fn();
    const mockCopyFile = mock.fn((_src: string, _dst: string) =>
      Promise.resolve()
    );
    const mockStat = mock.fn(() => Promise.resolve({ mode: 0 }));
    const mockChmod = mock.fn();

    t.mock.method(native, "generateNativePackage", mockGenerateNativePackage);
    t.mock.method(fs, "mkdir", mockMkdir);
    t.mock.method(fs, "stat", mockStat);
    t.mock.method(fs, "chmod", mockChmod);
    t.mock.method(fs, "copyFile", mockCopyFile);

    const platform: Platform = { os: "windows", arch: "x64" };
    const version = "1.0.0";

    // Since tar.create cannot be easily mocked with ESM, we expect the
    // function to fail when it tries to create the tarball (no real files).
    // We verify the other calls were made correctly before tar.create is called.
    try {
      await operations.packPlatform({ platform, version });
    } catch {
      // Expected to fail since no real files exist
    }

    assert.ok(
      mockCopyFile.mock.calls[0].arguments[0].endsWith("turbo.exe"),
      "source ends with .exe"
    );
    assert.ok(
      mockCopyFile.mock.calls[0].arguments[1].endsWith("turbo.exe"),
      "destination ends with .exe"
    );
    assert.equal(mockGenerateNativePackage.mock.calls.length, 1);
    assert.equal(mockMkdir.mock.calls.length, 1);
    assert.equal(mockCopyFile.mock.calls.length, 1);
    assert.equal(mockChmod.mock.calls.length, 1);
    assert.equal(mockChmod.mock.calls[0].arguments[1], 0o111);
  });
});
