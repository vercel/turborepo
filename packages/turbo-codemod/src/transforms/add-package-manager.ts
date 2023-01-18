import path from "path";
import fs from "fs-extra";

import getPackageManager from "../utils/getPackageManager";
import getPackageManagerVersion from "../utils/getPackageManagerVersion";
import getTransformerHelpers from "../utils/getTransformerHelpers";
import { TransformerResults } from "../runner";
import type { TransformerArgs } from "../types";

// transformer details
const TRANSFORMER = "add-package-manager";
const DESCRIPTION = "Set the `packageManager` key in root `package.json` file";
const INTRODUCED_IN = "1.1.0";

export function transformer({
  root,
  options,
}: TransformerArgs): TransformerResults {
  const { log, runner } = getTransformerHelpers({
    transformer: TRANSFORMER,
    rootPath: root,
    options,
  });

  log.info(`Set "packageManager" key in root "package.json" file...`);
  const packageManager = getPackageManager({ directory: root });
  if (!packageManager) {
    return runner.abortTransform({
      reason: `Unable to determine package manager for ${root}`,
    });
  }

  // handle workspaces...
  let version = null;
  try {
    version = getPackageManagerVersion(packageManager, root);
  } catch (err) {
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
  name: `${TRANSFORMER}: ${DESCRIPTION}`,
  value: TRANSFORMER,
  introducedIn: INTRODUCED_IN,
  transformer,
};

export default transformerMeta;
