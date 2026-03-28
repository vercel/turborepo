import path from "node:path";
import { getWorkspaceDetails, type Project } from "@turbo/workspaces";
import fs from "fs-extra";
import type { Transformer, TransformerArgs } from "../types";
import type { TransformerResults } from "../runner";
import { getTransformerHelpers } from "../utils/get-transformer-helpers";

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
    return (await fs.readJson(pkgJsonPath)) as { name?: string };
  } catch (e) {
    return null;
  }
}

export function getNewPkgName({
  pkgPath,
  pkgName
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
  options
}: TransformerArgs): Promise<TransformerResults> {
  const { log, runner } = getTransformerHelpers({
    transformer: TRANSFORMER,
    rootPath: root,
    options
  });

  log.info('Validating that each package has a unique "name"...');

  let project: Project;
  try {
    project = await getWorkspaceDetails({ root });
  } catch (e) {
    return runner.abortTransform({
      reason: `Unable to determine package manager for ${root}`
    });
  }

  const packagePaths: Array<string> = [project.paths.packageJson];
  const packagePromises: Array<Promise<PartialPackageJson | null>> = [
    readPkgJson(project.paths.packageJson)
  ];

  // add all workspace package.json files
  for (const workspace of project.workspaceData.workspaces) {
    const pkgJsonPath = workspace.paths.packageJson;
    packagePaths.push(pkgJsonPath);
    packagePromises.push(readPkgJson(pkgJsonPath));
  }

  // await, and then zip the paths and promise results together
  const packageContent = await Promise.all(packagePromises);
  const packageToContent = Object.fromEntries(
    packagePaths.map((pkgJsonPath, idx) => [pkgJsonPath, packageContent[idx]])
  );

  // Collect existing names and detect duplicates.
  const nameToPackages = new Map<string, Array<string>>();
  for (const [pkgJsonPath, pkgJsonContent] of Object.entries(
    packageToContent
  )) {
    if (pkgJsonContent?.name) {
      const existing = nameToPackages.get(pkgJsonContent.name) || [];
      existing.push(pkgJsonPath);
      nameToPackages.set(pkgJsonContent.name, existing);
    }
  }

  const duplicates = [...nameToPackages.entries()].filter(
    ([, paths]) => paths.length > 1
  );
  if (duplicates.length > 0) {
    const messages = duplicates.map(([name, paths]) => {
      const relativePaths = paths.map((p) => path.relative(root, p));
      return `  - "${name}" found in: ${relativePaths.join(", ")}`;
    });
    return runner.abortTransform({
      reason: `Found packages with duplicate "name" fields:\n${messages.join("\n")}\nPlease resolve these duplicates manually and re-run the codemod.`
    });
  }

  // Add names only to packages that are missing one.
  const existingNames = new Set(nameToPackages.keys());
  for (const [pkgJsonPath, pkgJsonContent] of Object.entries(
    packageToContent
  )) {
    if (pkgJsonContent && !pkgJsonContent.name) {
      const newName = getNewPkgName({
        pkgPath: pkgJsonPath,
        pkgName: undefined
      });
      runner.modifyFile({
        filePath: pkgJsonPath,
        after: {
          ...pkgJsonContent,
          name: newName
        }
      });
      existingNames.add(newName);
    }
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
