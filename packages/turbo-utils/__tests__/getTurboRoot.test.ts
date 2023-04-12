import path from "path";
import getTurboRoot from "../src/getTurboRoot";
import { setupTestFixtures } from "@turbo/test-utils";

describe("getTurboConfigs", () => {
  const { useFixture } = setupTestFixtures({
    directory: path.join(__dirname, "../"),
    test: "common",
  });

  test.each([[""], ["child"]])(
    "finds the root in a non-monorepo (%s)",
    (repoPath) => {
      const { root } = useFixture({ fixture: `single-package` });
      const turboRoot = getTurboRoot(path.join(root, repoPath));
      expect(turboRoot).toEqual(root);
    }
  );

  test.each([
    [""],
    ["apps"],
    ["apps/docs"],
    ["apps/web"],
    ["packages"],
    ["packages/ui"],
    ["not-a-real/path"],
  ])("finds the root in a monorepo with workspace configs (%s)", (repoPath) => {
    const { root } = useFixture({ fixture: `workspace-configs` });
    const turboRoot = getTurboRoot(path.join(root, repoPath));
    expect(turboRoot).toEqual(root);
  });
});
