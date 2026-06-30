interface DevEnginesPackageManager {
  name: string;
  version: string;
}

interface PackageJsonWithPackageManager {
  packageManager?: string;
  devEngines?: {
    packageManager?: DevEnginesPackageManager;
  };
}

export interface PackageManagerInfo {
  name: string;
  version: string;
}

export function getPackageManagerInfo(
  packageJson: PackageJsonWithPackageManager
): PackageManagerInfo {
  const devEnginesPackageManager = packageJson.devEngines?.packageManager;
  if (devEnginesPackageManager?.name && devEnginesPackageManager?.version) {
    return devEnginesPackageManager;
  }

  const packageManager = packageJson.packageManager;
  if (!packageManager) {
    throw new Error(
      "Missing package manager declaration. Expected `devEngines.packageManager` or legacy `packageManager`."
    );
  }

  const match = /^([^@]+)@(.+)$/.exec(packageManager);
  if (!match) {
    throw new Error(`Invalid packageManager value: ${packageManager}`);
  }

  return {
    name: match[1],
    version: match[2]
  };
}

export function getPackageManagerInstallCommand(
  packageManager: string,
  version: string,
  options: { updateLockfile?: boolean } = {}
): string | undefined {
  const { updateLockfile = false } = options;

  switch (packageManager) {
    case "pnpm": {
      const flags = updateLockfile ? " --no-frozen-lockfile" : "";
      return `corepack prepare pnpm@${version} --activate && pnpm install${flags}`;
    }
    case "npm": {
      const flags = updateLockfile ? " --force" : "";
      return `corepack prepare npm@${version} --activate && npm install${flags}`;
    }
    case "yarn": {
      const yarnMajorVersion = version.split(".")[0];
      if (yarnMajorVersion && parseInt(yarnMajorVersion, 10) >= 2) {
        const mode = updateLockfile ? " --mode update-lockfile" : "";
        return `corepack prepare yarn@${version} --activate && yarn install${mode}`;
      }
      return `corepack prepare yarn@${version} --activate && yarn install`;
    }
    case "bun":
      return "bun install";
    default:
      return undefined;
  }
}
