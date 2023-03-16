import chalk from "chalk";
import managers from "./managers";
import { Project, Options, PackageManagerDetails } from "./types";
import install from "./install";
import { Logger } from "./logger";
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
  to,
  logger,
  options,
}: {
  project: Project;
  to: PackageManagerDetails;
  logger: Logger;
  options?: Options;
}) {
  logger.header(
    `Converting project from ${project.packageManager} to ${to.name}.`
  );

  if (project.packageManager == to.name) {
    throw new ConvertError("You are already using this package manager", {
      type: "package_manager-already_in_use",
    });
  }

  // remove old workspace data
  await managers[project.packageManager].remove({
    project,
    to,
    logger,
    options,
  });

  // create new workspace data
  await managers[to.name].create({ project, to, logger, options });

  logger.mainStep("Installing dependencies");
  if (!options?.skipInstall) {
    await managers[to.name].convertLock({ project, logger, options });
    await install({ project, to, logger, options });
  } else {
    logger.subStep(chalk.yellow("Skipping install"));
  }

  logger.mainStep(`Cleaning up ${project.packageManager} workspaces`);
  await managers[project.packageManager].clean({ project, logger });
}
