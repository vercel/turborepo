import { getAvailablePackageManagers } from "turbo-utils";
import getWorkspaceDetails from "./getWorkspaceDetails";
import type { PackageManager, Options, InstallArgs, Workspace } from "./types";
import { default as convertProject } from "./convert";
import { Logger } from "./logger";
import install, { getPackageManagerMeta } from "./install";
import MANAGERS from "./managers";

async function convert({
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
  await convertProject({
    project,
    to: {
      name: to,
      version: availablePackageManagers[to].version as PackageManager,
    },
    logger,
    options,
  });
}

export type { PackageManager, Options, InstallArgs, Workspace };
export {
  convert,
  getWorkspaceDetails,
  install,
  MANAGERS,
  getPackageManagerMeta,
};
