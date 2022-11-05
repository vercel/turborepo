import path from "path";
import { Logger } from "../../src/logger";
import { default as npm } from "../../src/managers/npm";
import { validateWorkspace } from "../test-utils";

const FIXTURES = path.resolve(__dirname, "../../__fixtures__");

describe("npm", () => {
  it("detects npm workspaces", async () => {
    expect(
      npm.verify({
        workspaceRoot: path.join(FIXTURES, "npm-workspaces"),
      })
    ).toEqual(true);
  });

  // it("removes npm workspaces when moving to yarn", async () => {
  //   const project = npm.read({
  //     workspaceRoot: path.join(FIXTURES, "npm-workspaces"),
  //   });

  //   npm.remove({
  //     project,
  //     to: { name: "yarn", version: "1.22.19" },
  //     logger: new Logger({ interactive: false}),
  //   });
  // });

  // it("removes npm workspaces when moving to pnpm", async () => {
  //   const project = npm.read({
  //     workspaceRoot: path.join(FIXTURES, "npm-workspaces"),
  //   });

  //   npm.remove({
  //     project,
  //     to: { name: "pnpm", version: "7.12.1" },
  //     logger: new Logger({ interactive: false}),
  //   });
  // });

  it("reads npm workspaces into generic format", async () => {
    const project = npm.read({
      workspaceRoot: path.join(FIXTURES, "npm-workspaces"),
    });
    expect(project.name).toEqual("npm-workspaces");
    expect(project.packageManager).toEqual("npm");
    // paths
    expect(project.paths.root).toMatch(/^.*__fixtures__\/npm-workspaces$/);
    expect(project.paths.lockfile).toMatch(
      /^.*__fixtures__\/npm-workspaces\/package-lock.json$/
    );
    expect(project.paths.packageJson).toMatch(
      /^.*__fixtures__\/npm-workspaces\/package.json$/
    );
    // workspaceData
    expect(project.workspaceData.globs).toEqual(["apps/*", "packages/*"]);
    expect(project.workspaceData.workspaces).toHaveLength(4);
    project.workspaceData.workspaces.forEach((workspace) =>
      validateWorkspace("npm", workspace)
    );
  });
});
