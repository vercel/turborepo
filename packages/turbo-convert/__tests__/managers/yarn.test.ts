import path from "path";
import { default as yarn } from "../../src/managers/yarn";
import { validateWorkspace } from "../test-utils";

const FIXTURES = path.resolve(__dirname, "../../__fixtures__");

describe("yarn", () => {
  it("detects yarn workspaces", async () => {
    expect(
      yarn.verify({
        workspaceRoot: path.join(FIXTURES, "yarn-workspaces"),
      })
    ).toEqual(true);
  });

  it("reads yarn workspaces into generic format", async () => {
    const project = yarn.read({
      workspaceRoot: path.join(FIXTURES, "yarn-workspaces"),
    });
    expect(project.name).toEqual("yarn-workspaces");
    expect(project.packageManager).toEqual("yarn");
    // paths
    expect(project.paths.root).toMatch(/^.*__fixtures__\/yarn-workspaces$/);
    expect(project.paths.lockfile).toMatch(
      /^.*__fixtures__\/yarn-workspaces\/yarn.lock$/
    );
    expect(project.paths.packageJson).toMatch(
      /^.*__fixtures__\/yarn-workspaces\/package.json$/
    );
    // workspaceData
    expect(project.workspaceData.globs).toEqual(["apps/*", "packages/*"]);
    expect(project.workspaceData.workspaces).toHaveLength(4);
    project.workspaceData.workspaces.forEach((workspace) =>
      validateWorkspace("yarn", workspace)
    );
  });
});
