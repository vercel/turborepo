import { execSync, ExecSyncOptions } from "child_process";
import os from "os";
import path from "path";
import fs from "fs-extra";
import getWorkspaceImplementation, {
  WorkspaceImplementations,
  
} from "../../../utils/getWorkspaceImplementation";
import { exec } from "../utils";

function getGlobalBinaryPaths(): Record<
  WorkspaceImplementations,
  string | undefined
> {
  return {
    yarn: exec(`yarn global bin`, { cwd: os.homedir() }),
    npm: exec(`npm bin --global`, { cwd: os.homedir() }),
    pnpm: exec(`pnpm  bin --global`, { cwd: os.homedir() }),
  };
}

function getGlobalUpgradeCommand(
  packageManager: WorkspaceImplementations,
  to: string = "latest"
) {
  switch (packageManager) {
    case "yarn":
      return `yarn global install turbo@${to}`;
    case "npm":
      return `npm install -g turbo${to}`;
    case "pnpm":
      return `pnpm install -g turbo@${to}`;
  }
}

function getLocalUpgradeCommand({
  packageManager,
  installType,
  to = "latest",
}: {
  packageManager: WorkspaceImplementations;
  installType: "dependencies" | "devDependencies";
  to?: string;
}) {
  switch (packageManager) {
    case "yarn":
      return `yarn add turbo@${to} -W${
        installType === "dependencies" ? "" : " --dev"
      }`;
    case "npm":
      return `npm install turbo@${to}${
        installType === "dependencies" ? "" : " --save-dev"
      }`;
    case "pnpm":
      return `pnpm install turbo@${to} -w${
        installType === "dependencies" ? "" : " --save-dev"
      }`;
  }
}

function getInstallType({ directory }: { directory: string }) {
  // read package.json to make sure we have a reference to turbo
  const packageJsonPath = path.join(directory, "package.json");
  try {
    const packageJson = fs.readJsonSync(packageJsonPath);
    if (packageJson?.devDependencies?.["turbo"]) {
      return "devDependencies";
    }
    if (packageJson?.dependencies?.["turbo"]) {
      return "dependencies";
    }
  } catch (err) {
    console.error(`Unable to find package.json at ${packageJsonPath}`);
  }

  return undefined;
}

/*
  Finding the correct command to upgrade depends on two things:
  1. The package manager
  2. The install method (local or global)

  We try global first to let turbo handle the inference, then we try local.
*/
export default function getTurboUpgradeCommand({
  directory,
  to,
}: {
  directory: string;
  to?: string;
}) {
  const turboBinaryPathFromGlobal = exec(`turbo bin`, {
    cwd: directory,
    stdio: "pipe",
  });
  const packageManagerGlobalBinaryPaths = getGlobalBinaryPaths();

  const globalPackageManager = Object.keys(
    packageManagerGlobalBinaryPaths
  ).find((packageManager) => {
    const packageManagerBinPath =
      packageManagerGlobalBinaryPaths[
        packageManager as WorkspaceImplementations
      ];
    if (packageManagerBinPath && turboBinaryPathFromGlobal) {
      return turboBinaryPathFromGlobal.includes(packageManagerBinPath);
    }

    return false;
  }) as WorkspaceImplementations;

  if (turboBinaryPathFromGlobal && globalPackageManager) {
    // figure which package manager we need to upgrade
    return getGlobalUpgradeCommand(globalPackageManager, to);
  } else {
    const packageManager = getWorkspaceImplementation(directory);
    // we didn't find a global install, so we'll try to find a local one
    const installType = getInstallType({ directory });
    if (packageManager && installType) {
      return getLocalUpgradeCommand({ packageManager, installType, to });
    }
  }

  return undefined;
}
