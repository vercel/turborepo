import chalk from "chalk";
import os from "os";
import inquirer from "inquirer";
import { getWorkspaceDetails } from "@turbo/workspaces";

import getCurrentVersion from "./steps/getCurrentVersion";
import getLatestVersion from "./steps/getLatestVersion";
import getCodemodsForMigration from "./steps/getTransformsForMigration";
import checkGitStatus from "../../utils/checkGitStatus";
import directoryInfo from "../../utils/directoryInfo";
import getTurboUpgradeCommand from "./steps/getTurboUpgradeCommand";
import Runner from "../../runner/Runner";
import type { MigrateCommandArgument, MigrateCommandOptions } from "./types";
import looksLikeRepo from "../../utils/looksLikeRepo";
import { TransformerResults } from "../../runner";
import { shutdownDaemon } from "./steps/shutdownDaemon";
import { execSync } from "child_process";

function endMigration({
  message,
  success,
}: {
  message?: string;
  success: boolean;
}) {
  if (success) {
    console.log(chalk.bold(chalk.green("Migration completed")));
    if (message) {
      console.log(message);
    }
    return process.exit(0);
  }

  console.log(chalk.bold(chalk.red("Migration failed")));
  if (message) {
    console.log(message);
  }
  return process.exit(1);
}

/**
Migration is done in 4 steps:
  -- gather information
  1. find the version (x) of turbo to migrate from (if not specified)
  2. find the version (y) of turbo to migrate to (if not specified)
  3. determine which codemods need to be run to move from version x to version y
  -- action
  4. execute the codemods (serially, and in order)
  5. update the turbo version (optionally)
**/
export default async function migrate(
  directory: MigrateCommandArgument,
  options: MigrateCommandOptions
) {
  // check git status
  if (!options.dry) {
    checkGitStatus({ directory, force: options.force });
  }

  const answers = await inquirer.prompt<{
    directoryInput?: string;
  }>([
    {
      type: "input",
      name: "directoryInput",
      message: "Where is the root of the repo to migrate?",
      when: !directory,
      default: ".",
      validate: (directory: string) => {
        const { exists, absolute } = directoryInfo({ directory });
        if (exists) {
          return true;
        } else {
          return `Directory ${chalk.dim(`(${absolute})`)} does not exist`;
        }
      },
      filter: (directory: string) => directory.trim(),
    },
  ]);

  const { directoryInput: selectedDirectory = directory as string } = answers;
  const { exists, absolute: root } = directoryInfo({
    directory: selectedDirectory,
  });
  if (!exists) {
    return endMigration({
      success: false,
      message: `Directory ${chalk.dim(`(${root})`)} does not exist`,
    });
  }

  if (!looksLikeRepo({ directory: root })) {
    return endMigration({
      success: false,
      message: `Directory (${chalk.dim(
        root
      )}) does not appear to be a repository`,
    });
  }

  const project = await getWorkspaceDetails({ root });
  if (!project) {
    return endMigration({
      success: false,
      message: `Unable to read determine package manager details from ${chalk.dim(
        root
      )}`,
    });
  }

  // step 1
  const fromVersion = getCurrentVersion(project, options);
  if (!fromVersion) {
    return endMigration({
      success: false,
      message: `Unable to infer the version of turbo being used by ${project.name}`,
    });
  }

  // step 2
  let toVersion = options.to;
  try {
    toVersion = await getLatestVersion(options);
  } catch (err) {
    let message = "UNKNOWN_ERROR";
    if (err instanceof Error) {
      message = err.message;
    }
    return endMigration({
      success: false,
      message,
    });
  }

  if (!toVersion) {
    return endMigration({
      success: false,
      message: `Unable to fetch the latest version of turbo`,
    });
  }

  if (fromVersion === toVersion) {
    return endMigration({
      success: true,
      message: `Nothing to do, current version (${chalk.bold(
        fromVersion
      )}) is the same as the requested version (${chalk.bold(toVersion)})`,
    });
  }

  // step 3
  const codemods = getCodemodsForMigration({ fromVersion, toVersion });
  if (codemods.length === 0) {
    console.log(
      `No codemods required to migrate from ${fromVersion} to ${toVersion}`,
      os.EOL
    );
  }

  // shutdown the turbo daemon before running codemods and upgrading
  // the daemon can handle version mismatches, but we do this as an extra precaution
  if (!options.dry) {
    shutdownDaemon({ project });
  }

  // step 4
  console.log(
    `Upgrading turbo from ${chalk.bold(fromVersion)} to ${chalk.bold(
      toVersion
    )} (${
      codemods.length === 0
        ? "no codemods required"
        : `${codemods.length} required codemod${
            codemods.length === 1 ? "" : "s"
          }`
    })`,
    os.EOL
  );

  const results: Array<TransformerResults> = [];
  for (let [idx, codemod] of codemods.entries()) {
    console.log(
      `(${idx + 1}/${codemods.length}) ${chalk.bold(`Running ${codemod.name}`)}`
    );

    const result = await codemod.transformer({
      root: project.paths.root,
      options,
    });
    Runner.logResults(result);
    results.push(result);
  }

  const hasTransformError = results.some(
    (result) =>
      result.fatalError ||
      Object.keys(result.changes).some((key) => result.changes[key].error)
  );

  if (hasTransformError) {
    return endMigration({
      success: false,
      message: `Could not complete migration due to codemod errors. Please fix the errors and try again.`,
    });
  }

  // step 5

  // find the upgrade command, and run it
  const upgradeCommand = await getTurboUpgradeCommand({
    project,
    to: options.to,
  });

  if (!upgradeCommand) {
    return endMigration({
      success: false,
      message: "Unable to determine upgrade command",
    });
  }

  // install
  if (options.install) {
    if (options.dry) {
      console.log(
        `Upgrading turbo with ${chalk.bold(upgradeCommand)} ${chalk.dim(
          "(dry run)"
        )}`,
        os.EOL
      );
    } else {
      console.log(`Upgrading turbo with ${chalk.bold(upgradeCommand)}`, os.EOL);
      try {
        execSync(upgradeCommand, { stdio: "pipe", cwd: project.paths.root });
      } catch (err) {
        return endMigration({
          success: false,
          message: `Unable to upgrade turbo: ${err}`,
        });
      }
    }
  } else {
    console.log(`Upgrade turbo with ${chalk.bold(upgradeCommand)}`, os.EOL);
  }

  endMigration({ success: true });
}
