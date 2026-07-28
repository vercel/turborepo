import path from "node:path";
import fs from "fs-extra";
import { getWorkspaceDetails, type Project } from "@turbo/workspaces";
import { type PackageJson, getAvailablePackageManagers } from "@turbo/utils";
import { getTransformerHelpers } from "../utils/get-transformer-helpers";
import type { TransformerResults } from "../runner";
import type { Transformer, TransformerArgs } from "../types";

// transformer details
const TRANSFORMER = "add-package-manager";
const DESCRIPTION =
  "Set the `devEngines.packageManager` key in root `package.json` file";
const INTRODUCED_IN = "1.1.0";

interface DevEnginesPackageManager {
  name: string;
  version: string;
}

interface PackageJsonWithDevEngines extends PackageJson {
  devEngines?: Record<string, unknown> & {
    packageManager?: DevEnginesPackageManager;
  };
}

export async function transformer({
  root,
  options
}: TransformerArgs): Promise<TransformerResults> {
  const { log, runner } = getTransformerHelpers({
    transformer: TRANSFORMER,
    rootPath: root,
    options
  });

  const rootPackageJsonPath = path.join(root, "package.json");
  const rootPackageJson = fs.readJsonSync(
    rootPackageJsonPath
  ) as PackageJsonWithDevEngines;
  if ("packageManager" in rootPackageJson) {
    log.info(`"packageManager" already set in root "package.json"`);
    return runner.finish();
  }
  if (rootPackageJson.devEngines?.packageManager) {
    log.info(`"devEngines.packageManager" already set in root "package.json"`);
    return runner.finish();
  }

  log.info(
    `Set "devEngines.packageManager" key in root "package.json" file...`
  );
  let project: Project;
  try {
    project = await getWorkspaceDetails({ root });
  } catch (e) {
    return runner.abortTransform({
      reason: `Unable to determine package manager for ${root}`
    });
  }

  const availablePackageManagers = await getAvailablePackageManagers({
    projectRoot: root
  });
  const { packageManager } = project;
  const version = availablePackageManagers[packageManager];
  if (!version) {
    return runner.abortTransform({
      reason: `Unable to determine package manager version for ${root}`
    });
  }

  const allWorkspaces = [
    {
      name: "package.json",
      path: root,
      packageJson: {
        ...rootPackageJson,
        packageJsonPath: rootPackageJsonPath
      }
    }
  ];

  for (const workspace of allWorkspaces) {
    const { packageJsonPath, ...pkgJson } = workspace.packageJson;
    const newJson = {
      ...pkgJson,
      devEngines: {
        ...pkgJson.devEngines,
        packageManager: {
          name: packageManager,
          version
        }
      }
    };
    runner.modifyFile({
      filePath: packageJsonPath,
      after: newJson
    });
  }

  return runner.finish();
}

const transformerMeta: Transformer = {
  name: TRANSFORMER,
  description: DESCRIPTION,
  introducedIn: INTRODUCED_IN,
  transformer
};

// eslint-disable-next-line import/no-default-export -- transforms require default export
export default transformerMeta;
