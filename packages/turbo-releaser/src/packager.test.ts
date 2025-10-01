import { describe, it, mock } from "node:test";
import assert from "node:assert/strict";
import { packAndPublish } from "./packager";
import type { Platform } from "./types";
import operations from "./operations";

describe("packager", () => {
  describe("packAndPublish", () => {
    it("should pack and publish for all platforms when skipPublish is false", async (t) => {
      const mockPackPlatform = mock.fn(() =>
        Promise.resolve("/path/to/artifact.tgz")
      );
      const mockPublishArtifacts = mock.fn((_paths: Array<string>) =>
        Promise.resolve()
      );
      t.mock.method(operations, "packPlatform", mockPackPlatform);
      t.mock.method(operations, "publishArtifacts", mockPublishArtifacts);

      const platforms: Array<Platform> = [
        { os: "darwin", arch: "x64" },
        { os: "linux", arch: "arm64" },
      ];
      const version = "1.0.0";
      const npmTag = "latest";

      await packAndPublish({ platforms, version, skipPublish: false, npmTag });

      assert.equal(mockPackPlatform.mock.calls.length, 2);
      assert.equal(mockPublishArtifacts.mock.calls.length, 1);
      assert.deepEqual(mockPublishArtifacts.mock.calls[0].arguments, [
        ["/path/to/artifact.tgz", "/path/to/artifact.tgz"],
        "latest",
      ]);
    });

    it("should pack but not publish when skipPublish is true", async (t) => {
      const mockPackPlatform = mock.fn(() =>
        Promise.resolve("/path/to/artifact.tgz")
      );
      const mockPublishArtifacts = mock.fn();

      t.mock.method(operations, "packPlatform", mockPackPlatform);
      t.mock.method(operations, "publishArtifacts", mockPublishArtifacts);

      const platforms: Array<Platform> = [{ os: "darwin", arch: "x64" }];
      const version = "1.0.0";
      const npmTag = "latest";

      await packAndPublish({ platforms, version, skipPublish: true, npmTag });

      assert.equal(mockPackPlatform.mock.calls.length, 1);
      assert.equal(mockPublishArtifacts.mock.calls.length, 0);
    });
  });
});
