import path from "node:path";
import { getWorkspaceDetails, type Project } from "@turbo/workspaces";
import { readJson } from "fs-extra";
import type { Transformer, TransformerArgs } from "../types";
import type { TransformerResults } from "../runner";
import { getTransformerHelpers } from "../utils/getTransformerHelpers";

// transformer details
const TRANSFORMER = "add-package-names";
const DESCRIPTION = "Ensure all packages have a name in their package.json";
const INTRODUCED_IN = "2.0.0-canary.0";

interface PartialPackageJson {
  name?: string;
}

async function readPkgJson(
  pkgJsonPath: string
): Promise<PartialPackageJson | null> {
  try {
    return (await readJson(pkgJsonPath)) as { name?: string };
  } catch (e) {
    return null;
  }
}

export function getNewPkgName({
  pkgPath,
  pkgName,
}: {
  pkgPath: string;
  pkgName?: string;
}): string {
  // find the scope if it exists
  let scope = "";
  let name = pkgName;
  if (pkgName && pkgName.startsWith("@") && pkgName.includes("/")) {
    const parts = pkgName.split("/");
    scope = `${parts[0]}/`;
    name = parts[1];
  }

  const dirName = path.basename(path.dirname(pkgPath));
  if (pkgName) {
    return `${scope}${dirName}-${name}`;
  }

  return `${scope}${dirName}`;
}

export async function transformer({
  root,
  options,
}: TransformerArgs): Promise<TransformerResults> {
  const { log, runner } = getTransformerHelpers({
    transformer: TRANSFORMER,
    rootPath: root,
    options,
  });

  log.info('Validating that each package has a unique "name"...');

  let project: Project;
  try {
    project = await getWorkspaceDetails({ root });
  } catch (e) {
    return runner.abortTransform({
      reason: `Unable to determine package manager for ${root}`,
    });
  }

  const packagePaths: Array<string> = [project.paths.packageJson];
  const packagePromises: Array<Promise<PartialPackageJson | null>> = [
    readPkgJson(project.paths.packageJson),
  ];

  // add all workspace package.json files
  project.workspaceData.workspaces.forEach((workspace) => {
    const pkgJsonPath = workspace.paths.packageJson;
    packagePaths.push(pkgJsonPath);
    packagePromises.push(readPkgJson(pkgJsonPath));
  });

  // await, and then zip the paths and promise results together
  const packageContent = await Promise.all(packagePromises);
  const packageToContent = Object.fromEntries(
    packagePaths.map((pkgJsonPath, idx) => [pkgJsonPath, packageContent[idx]])
  );

  // wait for all package.json files to be read
  const names = new Set();
  for (const [pkgJsonPath, pkgJsonContent] of Object.entries(
    packageToContent
  )) {
    if (pkgJsonContent) {
      // name is missing or isn't unique
      if (!pkgJsonContent.name || names.has(pkgJsonContent.name)) {
        const newName = getNewPkgName({
          pkgPath: pkgJsonPath,
          pkgName: pkgJsonContent.name,
        });
        runner.modifyFile({
          filePath: pkgJsonPath,
          after: {
            ...pkgJsonContent,
            name: newName,
          },
        });
        names.add(newName);
      } else {
        names.add(pkgJsonContent.name);
      }
    }
  }

  return runner.finish();
}

const transformerMeta: Transformer = {
  name: TRANSFORMER,
  description: DESCRIPTION,
  introducedIn: INTRODUCED_IN,
  transformer,
};

// eslint-disable-next-line import/no-default-export -- transforms require default export
export default transformerMeta;
