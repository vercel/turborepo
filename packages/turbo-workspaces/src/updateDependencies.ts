import path from "node:path";
import { writeJSONSync } from "fs-extra";
import chalk from "chalk";
import type { DependencyList, DependencyGroups } from "@turbo/utils";
import type {
  Project,
  Workspace,
  AvailablePackageManagerDetails,
  Options,
} from "./types";
import type { Logger } from "./logger";
import { getPackageJson } from "./utils";

function updateDependencyList({
  dependencyList,
  project,
  to,
}: {
  dependencyList: DependencyList;
  project: Project;
  to: AvailablePackageManagerDetails;
}): { dependencyList: DependencyList; updated: Array<string> } {
  const updated: Array<string> = [];
  project.workspaceData.workspaces.forEach((workspace) => {
    const { name } = workspace;
    if (dependencyList[name]) {
      const workspaceVersion = dependencyList[name];
      const version = workspaceVersion.startsWith("workspace:")
        ? workspaceVersion.slice("workspace:".length)
        : workspaceVersion;
      dependencyList[name] =
        to.name === "pnpm" ? `workspace:${version}` : version;
      updated.push(name);
    }
  });

  return { dependencyList, updated };
}

/**
 * Convert workspace dependencies to the format that `to` requires. Only needed when pnpm is involved as
 * it requires `workspace:*` and all the rest support `*`
 */
export function updateDependencies({
  project,
  workspace,
  to,
  logger,
  options,
}: {
  workspace: Workspace;
  project: Project;
  to: AvailablePackageManagerDetails;
  logger: Logger;
  options?: Options;
}): void {
  // this step isn't required if moving between yarn / npm / bun
  if (
    ["yarn", "npm", "bun"].includes(to.name) &&
    ["yarn", "npm", "bun"].includes(project.packageManager)
  ) {
    return;
  }

  // update all dependencies
  const workspacePackageJson = getPackageJson({
    workspaceRoot: workspace.paths.root,
  });

  // collect stats as we go for consolidated output at the end
  const stats: Record<keyof DependencyGroups, Array<string>> = {
    dependencies: [],
    devDependencies: [],
    peerDependencies: [],
    optionalDependencies: [],
  };

  const allDependencyKeys: Array<keyof DependencyGroups> = [
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
  ];

  allDependencyKeys.forEach((depKey) => {
    const depList = workspacePackageJson[depKey];
    if (depList) {
      const { updated, dependencyList } = updateDependencyList({
        dependencyList: depList,
        project,
        to,
      });

      workspacePackageJson[depKey] = dependencyList;
      stats[depKey] = updated;
    }
  });

  const toLog = (key: keyof DependencyGroups) => {
    const total = stats[key].length;
    if (total > 0) {
      return `${chalk.green(total.toString())} ${key}`;
    }
    return undefined;
  };

  const allChanges = allDependencyKeys
    .map(toLog)
    .filter(Boolean) as Array<string>;
  const workspaceLocation = `./${path.relative(
    project.paths.root,
    workspace.paths.packageJson
  )}`;
  if (allChanges.length >= 1) {
    let logLine = "updating";
    allChanges.forEach((stat, idx) => {
      if (allChanges.length === 1) {
        logLine += ` ${stat} in ${workspaceLocation}`;
      } else if (idx === allChanges.length - 1) {
        logLine += `and ${stat} in ${workspaceLocation}`;
      } else {
        logLine += ` ${stat}, `;
      }
    });

    logger.workspaceStep(logLine);
  } else {
    logger.workspaceStep(
      `no workspace dependencies found in ${workspaceLocation}`
    );
  }

  if (!options?.dry) {
    writeJSONSync(workspace.paths.packageJson, workspacePackageJson, {
      spaces: 2,
    });
  }
}
