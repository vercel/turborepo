import path from "path";
import { default as pnpm } from "../../src/managers/pnpm";
import { validateWorkspace } from "../test-utils";

const FIXTURES = path.resolve(__dirname, "../../__fixtures__");

describe("pnpm", () => {
  it("detects pnpm workspaces", async () => {
    expect(
      pnpm.verify({
        workspaceRoot: path.join(FIXTURES, "pnpm-workspaces"),
      })
    ).toEqual(true);
  });

  it("reads pnpm workspaces into generic format", async () => {
    const project = pnpm.read({
      workspaceRoot: path.join(FIXTURES, "pnpm-workspaces"),
    });
    expect(project.name).toEqual("pnpm-workspaces");
    expect(project.packageManager).toEqual("pnpm");
    // paths
    expect(project.paths.root).toMatch(/^.*__fixtures__\/pnpm-workspaces$/);
    expect(project.paths.lockfile).toMatch(
      /^.*__fixtures__\/pnpm-workspaces\/pnpm-lock.yaml$/
    );
    expect(project.paths.packageJson).toMatch(
      /^.*__fixtures__\/pnpm-workspaces\/package.json$/
    );
    // workspaceData
    expect(project.workspaceData.globs).toEqual(["apps/*", "packages/*"]);
    expect(project.workspaceData.workspaces).toHaveLength(4);
    project.workspaceData.workspaces.forEach((workspace) =>
      validateWorkspace("pnpm", workspace)
    );
  });
});
