import chalk from "chalk";
import { MANAGERS } from "./managers";
import type {
  Project,
  Options,
  RequestedPackageManagerDetails,
  AvailablePackageManagerDetails,
} from "./types";
import { install } from "./install";
import type { Logger } from "./logger";
import { ConvertError } from "./errors";

/*
  * Convert a project using workspaces from one package manager to another.

  Steps are run in the following order:
  1. managerFrom.remove
  2. managerTo.create
  3. managerTo.convertLock
  3. install
  4. managerFrom.clean

*/
export async function convertProject({
  project,
  convertTo,
  logger,
  options,
}: {
  project: Project;
  convertTo: RequestedPackageManagerDetails;
  logger: Logger;
  options?: Options;
}) {
  logger.header(
    `Converting project from ${project.packageManager} to ${convertTo.name}.`
  );

  if (project.packageManager === convertTo.name) {
    throw new ConvertError("You are already using this package manager", {
      type: "package_manager-already_in_use",
    });
  }

  if (!convertTo.version) {
    throw new ConvertError(
      `${convertTo.name} is not installed, or could not be located`,
      {
        type: "package_manager-could_not_be_found",
      }
    );
  }

  // this cast is safe since we've just verified that the version exists above
  const to = convertTo as AvailablePackageManagerDetails;

  // remove old workspace data
  await MANAGERS[project.packageManager].remove({
    project,
    to,
    logger,
    options,
  });

  // create new workspace data
  await MANAGERS[to.name].create({ project, to, logger, options });

  logger.mainStep("Installing dependencies");
  if (!options?.skipInstall) {
    await MANAGERS[to.name].convertLock({ project, to, logger, options });
    await install({ project, to, logger, options });
  } else {
    logger.subStep(chalk.yellow("Skipping install"));
  }

  logger.mainStep(`Cleaning up ${project.packageManager} workspaces`);
  await MANAGERS[project.packageManager].clean({ project, logger });
}
