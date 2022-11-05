import path from "path";
import {
  isYarnAvailable,
  isPnpmAvailable,
  isNpmAvailable,
  PackageManagerAvailable,
} from "turbo-utils";
import getWorkspaceDetails from "./getWorkspaceDetails";
import { PackageManagers, Options } from "./types";
import { default as convert } from "./convert";
import { Logger } from "./logger";

// find all available package managers
const availablePackageManagers: Record<
  PackageManagers,
  PackageManagerAvailable
> = {
  yarn: isYarnAvailable(),
  pnpm: isPnpmAvailable(),
  npm: isNpmAvailable(),
};

async function convertMonorepo({
  root,
  to,
  options,
}: {
  root: string;
  to: PackageManagers;
  options?: Options;
}) {
  const logger = new Logger({ ...options, interactive: false });
  const workspaceRoot = path.isAbsolute(root)
    ? root
    : path.relative(process.cwd(), root);

  const project = getWorkspaceDetails({ workspaceRoot });
  if (to === project.packageManager) {
    throw new Error("You are already using this package manager");
  }

  await convert({
    project,
    to: {
      name: to,
      version: availablePackageManagers[to].version as PackageManagers,
    },
    logger,
    options,
  });
}

export { convertMonorepo, getWorkspaceDetails };
export { default as MANAGERS } from "./managers";
