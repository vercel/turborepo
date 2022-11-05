import fs from "fs-extra";
import chalk from "chalk";
import path from "path";
import glob from "fast-glob";
import yaml from "js-yaml";
import {
  Project,
  PackageJson,
  Workspace,
  DependencyList,
  PackageManagerDetails,
  Options,
} from "./types";
import { Logger } from "./logger";

const PACKAGE_JSON_REGEX =
  /(?<manager>npm|pnpm|yarn)@(?<version>\d+\.\d+\.\d+(-.+)?)/;

function delay(ms: number = 500) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function getPackageJson({
  workspaceRoot,
}: {
  workspaceRoot: string;
}): PackageJson {
  const packageJsonPath = path.join(workspaceRoot, "package.json");
  if (!fs.existsSync(packageJsonPath)) {
    throw new Error(`no "package.json" found at ${workspaceRoot}`);
  }
  const packageJson = JSON.parse(
    fs.readFileSync(packageJsonPath, "utf8")
  ) as PackageJson;
  return packageJson;
}

function getWorkspacePackageManager({
  workspaceRoot,
}: {
  workspaceRoot: string;
}): boolean | string {
  const { packageManager } = getPackageJson({ workspaceRoot });
  if (packageManager) {
    const { groups } = PACKAGE_JSON_REGEX.exec(packageManager) || {};
    if (groups) {
      return groups["manager"];
    }
  }
  return false;
}

function getWorkspaceName({
  workspaceRoot,
}: {
  workspaceRoot: string;
}): string {
  const packageJson = getPackageJson({ workspaceRoot });
  if (packageJson.name) {
    return packageJson.name as string;
  }
  const workspaceDirectory = path.basename(workspaceRoot);
  return workspaceDirectory;
}

function getPnpmWorkspaces({
  workspaceRoot,
}: {
  workspaceRoot: string;
}): Array<string> {
  const workspaceFile = path.join(workspaceRoot, "pnpm-workspace.yaml");
  if (fs.existsSync(workspaceFile)) {
    const workspaceConfig = yaml.load(
      fs.readFileSync(workspaceFile, "utf8")
    ) as { packages: Array<string> };
    return workspaceConfig.packages;
  }

  return [];
}

function expandWorkspaces({
  workspaceRoot,
  workspaceGlobs,
}: {
  workspaceRoot: string;
  workspaceGlobs?: string[];
}): Array<Workspace> {
  if (!workspaceGlobs) {
    return [];
  }
  return workspaceGlobs
    .flatMap((workspaceGlob) => {
      const workspacePackageJsonGlob = `${workspaceGlob}/package.json`;
      return glob.sync(workspacePackageJsonGlob, {
        onlyFiles: true,
        absolute: true,
        cwd: workspaceRoot,
      });
    })
    .map((workspacePackageJson) => {
      const workspaceRoot = path.dirname(workspacePackageJson);
      const name = getWorkspaceName({ workspaceRoot });
      return {
        name,
        paths: { root: workspaceRoot, packageJson: workspacePackageJson },
      };
    });
}

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
      dependencyList[name] = to.name === "pnpm" ? `workspace:*` : "*";
      updated.push(name);
    }
  });

  return { dependencyList, updated };
}

function updateDependencies({
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
}) {
  const workspacePackageJson = getPackageJson({
    workspaceRoot: workspace.paths.root,
  });

  // update all dependencies
  const stats: Record<string, Array<string>> = {
    dependencies: [],
    devDependencies: [],
  };
  const { devDependencies, dependencies } = workspacePackageJson;
  if (devDependencies) {
    const { updated, dependencyList } = updateDependencyList({
      dependencyList: devDependencies,
      project,
      to,
    });
    workspacePackageJson["devDependencies"] = dependencyList;
    stats.devDependencies = updated;
  }
  if (dependencies) {
    const { updated, dependencyList } = updateDependencyList({
      dependencyList: dependencies,
      project,
      to,
    });
    workspacePackageJson["dependencies"] = dependencyList;
    stats.dependencies = updated;
  }

  const numDevDependencies = stats.devDependencies.length;
  const numDependencies = stats.dependencies.length;
  const suffix = (num: number) => `${num <= 1 ? "y" : "ies"}`;

  // both updated
  if (numDependencies > 0 && numDevDependencies > 0) {
    logger.workspaceStep(
      `updating ${chalk.green(
        numDevDependencies.toString()
      )} devDependenc${suffix(numDependencies)} and ${chalk.green(
        numDependencies.toString()
      )} dependenc${suffix(numDependencies)} in ./${path.relative(
        project.paths.root,
        workspace.paths.packageJson
      )}`
    );
  }
  // only devDependencies updated
  else if (numDevDependencies > 0) {
    logger.workspaceStep(
      `updating ${chalk.green(
        numDevDependencies.toString()
      )} devDependenc${suffix(numDevDependencies)} in ./${path.relative(
        project.paths.root,
        workspace.paths.packageJson
      )}`
    );
  }
  // only dependencies updated
  else if (numDependencies > 0) {
    logger.workspaceStep(
      `updating ${chalk.green(numDependencies.toString())} dependenc${suffix(
        numDependencies
      )} in ./${path.relative(project.paths.root, workspace.paths.packageJson)}`
    );
    // no dependencies updated
  } else {
    logger.workspaceStep(
      `no workspace dependencies found in ./${path.relative(
        project.paths.root,
        workspace.paths.packageJson
      )}`
    );
  }

  if (!options?.dry) {
    fs.writeJSONSync(workspace.paths.packageJson, workspacePackageJson, {
      spaces: 2,
    });
  }
}

export {
  delay,
  getPackageJson,
  getWorkspacePackageManager,
  getWorkspaceName,
  expandWorkspaces,
  getPnpmWorkspaces,
  updateDependencyList,
  updateDependencies,
};
