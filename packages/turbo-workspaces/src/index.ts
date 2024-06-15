import { getAvailablePackageManagers, type PackageManager } from "@turbo/utils";
import { getWorkspaceDetails } from "./getWorkspaceDetails";
import { convertProject } from "./convert";
import { Logger } from "./logger";
import { install, getPackageManagerMeta } from "./install";
import { ConvertError } from "./errors";
import { MANAGERS } from "./managers";
import type { Options, InstallArgs, Workspace, Project } from "./types";
import type { ConvertErrorType } from "./errors";

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
    convertTo: {
      name: to,
      version: availablePackageManagers[to],
    },
    logger,
    options,
  });
}

export type { Options, InstallArgs, Workspace, Project, ConvertErrorType };
export {
  convert,
  getWorkspaceDetails,
  install,
  MANAGERS,
  getPackageManagerMeta,
  ConvertError,
};
