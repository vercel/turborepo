import type { Project } from "../src/types";
import { isCompatibleWithBunWorkspaces } from "../src/utils";

describe("utils", () => {
  describe("isCompatibleWithBunWorkspace", () => {
    test.each([
      { globs: ["apps/*"], expected: true },
      { globs: ["apps/*", "packages/*"], expected: true },
      { globs: ["*"], expected: true },
      { globs: ["workspaces/**/*"], expected: false },
      { globs: ["apps/*", "packages/**/*"], expected: false },
      { globs: ["apps/*", "packages/*/utils/*"], expected: false },
      { globs: ["internal-*/*"], expected: false },
    ])("should return $result when given %globs", ({ globs, expected }) => {
      const result = isCompatibleWithBunWorkspaces({
        project: {
          workspaceData: { globs },
        } as Project,
      });
      expect(result).toEqual(expected);
    });
  });
});
