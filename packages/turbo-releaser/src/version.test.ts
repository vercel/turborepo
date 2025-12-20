import { describe, it, mock } from "node:test";
import assert from "node:assert";
import fs from "node:fs/promises";
import { getVersionInfo } from "./version";

describe("getVersionInfo", () => {
  it("should read version and npm tag from version.txt", async (t) => {
    const mockReadFile = mock.fn((_path, _encoding) => {
      return Promise.resolve("1.0.0\nbeta\n");
    });
    t.mock.method(fs, "readFile", mockReadFile);
    const result = await getVersionInfo("some-path/version.txt");
    assert.deepStrictEqual(result, { version: "1.0.0", npmTag: "beta" });
    assert.equal(
      mockReadFile.mock.calls[0].arguments[0],
      "some-path/version.txt"
    );
  });

  it("should throw an error if version.txt is not found", async (t) => {
    const mockReadFile = mock.fn((_path, _encoding) => {
      return Promise.reject(new Error("File not found"));
    });
    t.mock.method(fs, "readFile", mockReadFile);
    await assert.rejects(() => getVersionInfo("version.txt"), {
      message: "File not found",
    });
  });
});
