import { PackageManager, Workspace } from "../src/types";

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
  testCase: Array<[PackageManager, string, Array<string> | undefined]>
): Array<
  [PackageManager, string, Array<string> | undefined, boolean, boolean, boolean]
> {
  const interactive = [true, false];
  const dryRun = [true, false];
  const withNodeModules = [true, false];

  const matrix: Array<
    [
      PackageManager,
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
  [PackageManager, boolean, boolean]
> {
  const packageManagers: Array<PackageManager> = ["pnpm", "npm", "yarn"];
  const interactive = [true, false];
  const dryRun = [true, false];

  const matrix: Array<[PackageManager, boolean, boolean]> = [];
  for (const p of packageManagers) {
    for (const i of interactive) {
      for (const d of dryRun) {
        matrix.push([p, i, d]);
      }
    }
  }

  return matrix;
}

export function generateArgMatrix() {
  const interactive = [true, false];
  const dryRun = [true, false];

  const matrix: Array<[boolean, boolean]> = [];
  for (const i of interactive) {
    for (const d of dryRun) {
      matrix.push([i, d]);
    }
  }

  return matrix;
}

export function generateConvertMatrix() {
  const packageManagers: Array<PackageManager> = ["pnpm", "npm", "yarn"];
  const interactive = [true, false];
  const dryRun = [true, false];
  const install = [true, false];

  const matrix: Array<
    [PackageManager, PackageManager, boolean, boolean, boolean]
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
