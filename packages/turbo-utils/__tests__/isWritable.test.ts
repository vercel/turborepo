import path from "node:path";
import fs from "fs-extra";
import { setupTestFixtures } from "@turbo/test-utils";
import { describe, it, expect, jest } from "@jest/globals";
import { isWriteable } from "../src/isWriteable";

describe("isWriteable", () => {
  const { useFixture } = setupTestFixtures({
    directory: path.join(__dirname, "../"),
    options: { emptyFixture: true },
  });

  it("correctly identifies a writeable directory", async () => {
    const { root } = useFixture({ fixture: `is-writeable` });
    const result = await isWriteable(root);
    expect(result).toEqual(true);
  });

  it("correctly identifies a non-writeable directory", async () => {
    const { root } = useFixture({ fixture: `is-writeable` });
    const result = await isWriteable(path.join(root, "does-not-exist"));
    expect(result).toEqual(false);
  });

  it("returns false on unexpected failure", async () => {
    const { root } = useFixture({ fixture: `is-writeable` });
    const mockFsAccess = jest
      .spyOn(fs, "access")
      .mockRejectedValue(new Error("unknown error"));

    const result = await isWriteable(root);
    expect(result).toEqual(false);
    expect(mockFsAccess).toHaveBeenCalledWith(root, fs.constants.W_OK);

    mockFsAccess.mockRestore();
  });
});
