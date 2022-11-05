import { PackageManagers, Workspace } from "../src/types";

export const validateWorkspace = (
  manager: PackageManagers,
  workspace: Workspace
) => {
  const type = ["web", "docs"].includes(workspace.name) ? "apps" : "packages";
  expect(workspace.paths.packageJson).toMatch(
    new RegExp(
      `^.*__fixtures__\/${manager}-workspaces\/${type}\/${workspace.name}\/package.json$`
    )
  );
  expect(workspace.paths.root).toMatch(
    new RegExp(
      `^.*__fixtures__\/${manager}-workspaces\/${type}\/${workspace.name}$`
    )
  );
};
