import { PackageManagers, Workspace } from "../src/types";

export const validateWorkspace = (
  fixtureDirectory: string,
  workspace: Workspace
) => {
  const type = ["web", "docs"].includes(workspace.name) ? "apps" : "packages";
  expect(workspace.paths.packageJson).toMatch(
    new RegExp(
      `^.*${fixtureDirectory}\/${type}\/${workspace.name}\/package.json$`
    )
  );
  expect(workspace.paths.root).toMatch(
    new RegExp(`^.*${fixtureDirectory}\/${type}\/${workspace.name}$`)
  );
};

export function extendMatrix(
  testCase: Array<[PackageManagers, string, Array<string> | undefined]>
): Array<
  [
    PackageManagers,
    string,
    Array<string> | undefined,
    boolean,
    boolean,
    boolean
  ]
> {
  const interactive = [true, false];
  const dryRun = [true, false];
  const withNodeModules = [true, false];

  const matrix: Array<
    [
      PackageManagers,
      string,
      Array<string> | undefined,
      boolean,
      boolean,
      boolean
    ]
  > = [];
  for (const t of testCase) {
    for (const i of interactive) {
      for (const d of dryRun) {
        for (const n of withNodeModules) {
          matrix.push([...t, n, i, d]);
        }
      }
    }
  }
  return matrix;
}

export function generateTestMatrix(): Array<
  [PackageManagers, boolean, boolean]
> {
  const packageManagers: Array<PackageManagers> = ["pnpm", "npm", "yarn"];
  const interactive = [true, false];
  const dryRun = [true, false];

  const matrix: Array<[PackageManagers, boolean, boolean]> = [];
  for (const p of packageManagers) {
    for (const i of interactive) {
      for (const d of dryRun) {
        matrix.push([p, i, d]);
      }
    }
  }

  return matrix;
}

export function generateConvertMatrix() {
  const packageManagers: Array<PackageManagers> = ["pnpm", "npm", "yarn"];
  const interactive = [true, false];
  const dryRun = [true, false];
  const install = [true, false];

  const matrix: Array<
    [PackageManagers, PackageManagers, boolean, boolean, boolean]
  > = [];
  for (const p1 of packageManagers) {
    for (const p2 of packageManagers) {
      for (const i of interactive) {
        for (const d of dryRun) {
          for (const inst of install) {
            matrix.push([p1, p2, i, d, inst]);
          }
        }
      }
    }
  }

  return matrix;
}
