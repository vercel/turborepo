import path from "path";
import fs from "fs-extra";

import getTransformerHelpers from "../utils/getTransformerHelpers";
import { TransformerResults } from "../runner";
import type { TransformerArgs } from "../types";
import { Project, getWorkspaceDetails } from "@turbo/workspaces";
import { getAvailablePackageManagers } from "@turbo/utils";

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
  const rootPackageJson = fs.readJsonSync(rootPackageJsonPath);
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

export default transformerMeta;
