import path from "node:path";
import { readJsonSync, existsSync } from "fs-extra";
import { gte } from "semver";
import {
  getAvailablePackageManagers,
  getPackageManagersBinPaths,
  logger,
  type PackageManager,
  type PackageJson,
} from "@turbo/utils";
import type { Project } from "@turbo/workspaces";
import { exec } from "../utils";

type InstallType = "dependencies" | "devDependencies";

function getGlobalUpgradeCommand(
  packageManager: PackageManager,
  to = "latest"
) {
  switch (packageManager) {
    case "yarn":
      return `yarn global add turbo@${to}`;
    case "npm":
      return `npm install turbo@${to} --global`;
    case "pnpm":
      return `pnpm add turbo@${to} --global`;
    case "bun":
      return `bun add turbo@${to} --global`;
  }
}

function getLocalUpgradeCommand({
  packageManager,
  packageManagerVersion,
  installType,
  isUsingWorkspaces,
  to = "latest",
}: {
  packageManager: PackageManager;
  packageManagerVersion: string;
  installType: InstallType;
  isUsingWorkspaces?: boolean;
  to?: string;
}) {
  const renderCommand = (
    command: Array<string | boolean | undefined>
  ): string => command.filter(Boolean).join(" ");
  switch (packageManager) {
    // yarn command differs depending on the version
    case "yarn":
      // yarn 2.x and 3.x (berry)
      if (gte(packageManagerVersion, "2.0.0")) {
        return renderCommand([
          "yarn",
          "add",
          `turbo@${to}`,
          installType === "devDependencies" && "--dev",
        ]);
        // yarn 1.x
      }
      return renderCommand([
        "yarn",
        "add",
        `turbo@${to}`,
        installType === "devDependencies" && "--dev",
        isUsingWorkspaces && "-W",
      ]);

    case "npm":
      return renderCommand([
        "npm",
        "install",
        `turbo@${to}`,
        installType === "devDependencies" && "--save-dev",
      ]);
    case "pnpm":
      return renderCommand([
        "pnpm",
        "add",
        `turbo@${to}`,
        installType === "devDependencies" && "--save-dev",
        isUsingWorkspaces && "-w",
      ]);
    case "bun":
      return renderCommand([
        "bun",
        "add",
        `turbo@${to}`,
        installType === "devDependencies" && "--dev",
      ]);
  }
}

function getInstallType({ root }: { root: string }): InstallType | undefined {
  // read package.json to make sure we have a reference to turbo
  const packageJsonPath = path.join(root, "package.json");
  if (!existsSync(packageJsonPath)) {
    logger.error(`Unable to find package.json at ${packageJsonPath}`);
    return undefined;
  }

  const packageJson = readJsonSync(packageJsonPath) as PackageJson;
  const isDevDependency =
    packageJson.devDependencies && "turbo" in packageJson.devDependencies;
  const isDependency =
    packageJson.dependencies && "turbo" in packageJson.dependencies;

  if (isDependency || isDevDependency) {
    return isDependency ? "dependencies" : "devDependencies";
  }

  return undefined;
}

/**
  Finding the correct command to upgrade depends on two things:
  1. The package manager
  2. The install method (local or global)

  We try global first to let turbo handle the inference, then we try local.
**/
export async function getTurboUpgradeCommand({
  project,
  to,
}: {
  project: Project;
  to?: string;
}) {
  const turboBinaryPathFromGlobal = exec(`turbo bin`, {
    cwd: project.paths.root,
    stdio: "pipe",
  });
  const packageManagerGlobalBinaryPaths = await getPackageManagersBinPaths();
  const globalPackageManager = Object.keys(
    packageManagerGlobalBinaryPaths
  ).find((packageManager) => {
    const packageManagerBinPath =
      packageManagerGlobalBinaryPaths[packageManager as PackageManager];
    if (packageManagerBinPath && turboBinaryPathFromGlobal) {
      return turboBinaryPathFromGlobal.includes(packageManagerBinPath);
    }

    return false;
  }) as PackageManager | undefined;

  if (turboBinaryPathFromGlobal && globalPackageManager) {
    // figure which package manager we need to upgrade
    return getGlobalUpgradeCommand(globalPackageManager, to);
  }
  const { packageManager } = project;
  // we didn't find a global install, so we'll try to find a local one
  const isUsingWorkspaces = project.workspaceData.globs.length > 0;
  const installType = getInstallType({ root: project.paths.root });
  const availablePackageManagers = await getAvailablePackageManagers();
  const version = availablePackageManagers[packageManager];

  if (version && installType) {
    return getLocalUpgradeCommand({
      packageManager,
      packageManagerVersion: version,
      installType,
      isUsingWorkspaces,
      to,
    });
  }

  return undefined;
}
