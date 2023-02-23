import { getAvailablePackageManagers } from "turbo-utils";
import getWorkspaceDetails from "./getWorkspaceDetails";
import { PackageManager, Options } from "./types";
import { default as convert } from "./convert";
import { Logger } from "./logger";

async function convertMonorepo({
  root,
  to,
  options,
}: {
  root: string;
  to: PackageManager;
  options?: Options;
}) {
  const logger = new Logger({ ...options, interactive: false });
  const [project, availablePackageManagers] = await Promise.all([
    getWorkspaceDetails({ root }),
    getAvailablePackageManagers(),
  ]);
  await convert({
    project,
    to: {
      name: to,
      version: availablePackageManagers[to].version as PackageManager,
    },
    logger,
    options,
  });
}

export { convertMonorepo, getWorkspaceDetails };
export { default as MANAGERS } from "./managers";
