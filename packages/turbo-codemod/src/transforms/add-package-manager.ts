import path from "node:path";
import { readJsonSync } from "fs-extra";
import { getWorkspaceDetails, type Project } from "@turbo/workspaces";
import { type PackageJson, getAvailablePackageManagers } from "@turbo/utils";
import { getTransformerHelpers } from "../utils/getTransformerHelpers";
import type { TransformerResults } from "../runner";
import type { TransformerArgs } from "../types";

// transformer details
const TRANSFORMER = "add-package-manager";
const DESCRIPTION = "Set the `packageManager` key in root `package.json` file";
const INTRODUCED_IN = "1.1.0";

export async function transformer({
  root,
  options,
}: TransformerArgs): Promise<TransformerResults> {
  const { log, runner } = getTransformerHelpers({
    transformer: TRANSFORMER,
    rootPath: root,
    options,
  });

  log.info(`Set "packageManager" key in root "package.json" file...`);
  let project: Project;
  try {
    project = await getWorkspaceDetails({ root });
  } catch (e) {
    return runner.abortTransform({
      reason: `Unable to determine package manager for ${root}`,
    });
  }

  const availablePackageManagers = await getAvailablePackageManagers();
  const { packageManager } = project;
  const version = availablePackageManagers[packageManager];
  if (!version) {
    return runner.abortTransform({
      reason: `Unable to determine package manager version for ${root}`,
    });
  }

  const pkgManagerString = `${packageManager}@${version}`;
  const rootPackageJsonPath = path.join(root, "package.json");
  const rootPackageJson = readJsonSync(rootPackageJsonPath) as PackageJson;
  const allWorkspaces = [
    {
      name: "package.json",
      path: root,
      packageJson: {
        ...rootPackageJson,
        packageJsonPath: rootPackageJsonPath,
      },
    },
  ];

  for (const workspace of allWorkspaces) {
    const { packageJsonPath, ...pkgJson } = workspace.packageJson;
    const newJson = { ...pkgJson, packageManager: pkgManagerString };
    runner.modifyFile({
      filePath: packageJsonPath,
      after: newJson,
    });
  }

  return runner.finish();
}

const transformerMeta = {
  name: TRANSFORMER,
  description: DESCRIPTION,
  introducedIn: INTRODUCED_IN,
  transformer,
};

// eslint-disable-next-line import/no-default-export -- transforms require default export
export default transformerMeta;
