import path from "node:path";
import fs from "fs-extra";
import { setupTestFixtures } from "@turbo/test-utils";
import { describe, it, expect } from "@jest/globals";
import { isFolderEmpty } from "../src/isFolderEmpty";

describe("isFolderEmpty", () => {
  const { useFixture } = setupTestFixtures({
    directory: path.join(__dirname, "../"),
    options: {
      emptyFixture: true,
    },
  });

  it("correctly identifies an empty directory", async () => {
    const { root } = useFixture({ fixture: `is-folder-empty` });
    const result = isFolderEmpty(root);
    expect(result.isEmpty).toEqual(true);
    expect(result.conflicts).toEqual([]);
  });

  it("correctly identifies a directory with non-conflicting files", async () => {
    const { root } = useFixture({ fixture: `is-folder-empty` });
    fs.writeFileSync(path.join(root, "LICENSE"), "MIT");
    const result = isFolderEmpty(root);
    expect(result.isEmpty).toEqual(true);
    expect(result.conflicts).toEqual([]);
  });

  it("correctly identifies a directory non-conflicting files (intelliJ)", async () => {
    const { root } = useFixture({ fixture: `is-folder-empty` });
    fs.writeFileSync(path.join(root, "intellij-idea-config.iml"), "{}");
    const result = isFolderEmpty(root);
    expect(result.isEmpty).toEqual(true);
    expect(result.conflicts).toEqual([]);
  });

  it("correctly identifies a directory conflicting files", async () => {
    const { root } = useFixture({ fixture: `is-folder-empty` });
    fs.writeFileSync(path.join(root, "README.md"), "my cool project");
    const result = isFolderEmpty(root);
    expect(result.isEmpty).toEqual(false);
    expect(result.conflicts).toEqual(["README.md"]);
  });
});
