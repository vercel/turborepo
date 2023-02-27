import os from "os";
import path from "path";
import fs from "fs-extra";
import { gte } from "semver";

import { exec } from "../utils";
import getPackageManager, {
  PackageManager,
} from "../../../utils/getPackageManager";
import getPackageManagerVersion from "../../../utils/getPackageManagerVersion";

type InstallType = "dependencies" | "devDependencies";

function getGlobalBinaryPaths(): Record<PackageManager, string | undefined> {
  return {
    // we run these from a tmpdir to avoid corepack interference
    yarn: exec(`yarn global bin`, { cwd: os.tmpdir() }),
    npm: exec(`npm bin --global`, { cwd: os.tmpdir() }),
    pnpm: exec(`pnpm  bin --global`, { cwd: os.tmpdir() }),
  };
}

function getGlobalUpgradeCommand(
  packageManager: PackageManager,
  to: string = "latest"
) {
  switch (packageManager) {
    case "yarn":
      return `yarn global add turbo@${to}`;
    case "npm":
      return `npm install turbo@${to} --global`;
    case "pnpm":
      return `pnpm install turbo@${to} --global`;
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
      } else {
        return renderCommand([
          "yarn",
          "add",
          `turbo@${to}`,
          installType === "devDependencies" && "--dev",
          isUsingWorkspaces && "-W",
        ]);
      }
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
        "install",
        `turbo@${to}`,
        installType === "devDependencies" && "--save-dev",
        isUsingWorkspaces && "-w",
      ]);
  }
}

function getInstallType({ directory }: { directory: string }): {
  installType?: InstallType;
  isUsingWorkspaces?: boolean;
} {
  // read package.json to make sure we have a reference to turbo
  const packageJsonPath = path.join(directory, "package.json");
  const pnpmWorkspaceConfig = path.join(directory, "pnpm-workspace.yaml");
  const isPnpmWorkspaces = fs.existsSync(pnpmWorkspaceConfig);

  if (!fs.existsSync(packageJsonPath)) {
    console.error(`Unable to find package.json at ${packageJsonPath}`);
    return { installType: undefined, isUsingWorkspaces: undefined };
  }

  const packageJson = fs.readJsonSync(packageJsonPath);
  const isDevDependency =
    packageJson.devDependencies && "turbo" in packageJson.devDependencies;
  const isDependency =
    packageJson.dependencies && "turbo" in packageJson.dependencies;
  let isUsingWorkspaces = "workspaces" in packageJson || isPnpmWorkspaces;

  if (isDependency || isDevDependency) {
    return {
      installType: isDependency ? "dependencies" : "devDependencies",
      isUsingWorkspaces,
    };
  }

  return {
    installType: undefined,
    isUsingWorkspaces,
  };
}

/**
  Finding the correct command to upgrade depends on two things:
  1. The package manager
  2. The install method (local or global)

  We try global first to let turbo handle the inference, then we try local.
**/
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
      packageManagerGlobalBinaryPaths[packageManager as PackageManager];
    if (packageManagerBinPath && turboBinaryPathFromGlobal) {
      return turboBinaryPathFromGlobal.includes(packageManagerBinPath);
    }

    return false;
  }) as PackageManager;

  if (turboBinaryPathFromGlobal && globalPackageManager) {
    // figure which package manager we need to upgrade
    return getGlobalUpgradeCommand(globalPackageManager, to);
  } else {
    const packageManager = getPackageManager({ directory });
    // we didn't find a global install, so we'll try to find a local one
    const { installType, isUsingWorkspaces } = getInstallType({ directory });
    if (packageManager && installType) {
      const packageManagerVersion = getPackageManagerVersion(
        packageManager,
        directory
      );

      return getLocalUpgradeCommand({
        packageManager,
        packageManagerVersion,
        installType,
        isUsingWorkspaces,
        to,
      });
    }
  }

  return undefined;
}
