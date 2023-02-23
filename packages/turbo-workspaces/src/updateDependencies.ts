import fs from "fs-extra";
import chalk from "chalk";
import path from "path";
import {
  Project,
  Workspace,
  DependencyList,
  PackageManagerDetails,
  Options,
  PackageJsonDependencies,
} from "./types";
import { Logger } from "./logger";
import { getPackageJson } from "./utils";

function updateDependencyList({
  dependencyList,
  project,
  to,
}: {
  dependencyList: DependencyList;
  project: Project;
  to: PackageManagerDetails;
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

export default function updateDependencies({
  project,
  workspace,
  to,
  logger,
  options,
}: {
  workspace: Workspace;
  project: Project;
  to: PackageManagerDetails;
  logger: Logger;
  options?: Options;
}): void {
  // this step isn't required if moving between yarn / npm
  if (
    ["yarn", "npm"].includes(to.name) &&
    ["yarn", "npm"].includes(project.packageManager)
  ) {
    return;
  }

  // update all dependencies
  const workspacePackageJson = getPackageJson({
    workspaceRoot: workspace.paths.root,
  });

  // collect stats as we go for consolidated output at the end
  const stats: Record<keyof PackageJsonDependencies, Array<string>> = {
    dependencies: [],
    devDependencies: [],
    peerDependencies: [],
    optionalDependencies: [],
  };

  const allDependencyKeys: Array<keyof PackageJsonDependencies> = [
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

  const toLog = (key: keyof PackageJsonDependencies) => {
    const total = stats[key].length;
    if (total > 0) {
      return `${chalk.green(total.toString())} ${key}`;
    }
    return undefined;
  };

  const allChanges = allDependencyKeys.map(toLog).filter(Boolean);
  const workspaceLocation = `./${path.relative(
    project.paths.root,
    workspace.paths.packageJson
  )}`;
  if (allChanges.length >= 1) {
    let logLine = "updating";
    allChanges.forEach((stat, idx) => {
      if (allChanges.length === 1) {
        logLine += ` ${stat} in ${workspaceLocation}`;
      } else {
        if (idx === allChanges.length - 1) {
          logLine += `and ${stat} in ${workspaceLocation}`;
        } else {
          logLine += ` ${stat}, `;
        }
      }
    });

    logger.workspaceStep(logLine);
  } else {
    logger.workspaceStep(
      `no workspace dependencies found in ${workspaceLocation}`
    );
  }

  if (!options?.dry) {
    fs.writeJSONSync(workspace.paths.packageJson, workspacePackageJson, {
      spaces: 2,
    });
  }
}
