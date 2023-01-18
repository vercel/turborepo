import chalk from "chalk";
import os from "os";
import inquirer from "inquirer";
import { execSync } from "child_process";

import getCurrentVersion from "./steps/getCurrentVersion";
import getLatestVersion from "./steps/getLatestVersion";
import getCodemodsForMigration from "./steps/getTransformsForMigration";
import checkGitStatus from "../../utils/checkGitStatus";
import directoryInfo from "../../utils/directoryInfo";
import getTurboUpgradeCommand from "./steps/getTurboUpgradeCommand";
import Runner from "../../runner/Runner";
import type { MigrateCommandArgument, MigrateCommandOptions } from "./types";

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

/*
Migration is done in 4 steps:
  -- gather information
  1. find the version (x) of turbo to migrate from (if not specified)
  2. find the version (y) of turbo to migrate to (if not specified)
  3. determine which codemods need to be run to move from version x to version y
  -- action
  4. execute the codemods (serially, and in order)
  5. update the turbo version (optionally)

*/
export default async function migrate(
  directory: MigrateCommandArgument,
  options: MigrateCommandOptions
) {
  // check git status
  if (!options.dry) {
    checkGitStatus(options.force);
  }

  const answers = await inquirer.prompt<{
    directoryInput?: string;
  }>([
    {
      type: "input",
      name: "directoryInput",
      message: "Where is the root of the repo where the transform should run?",
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

  // step 1
  const fromVersion = getCurrentVersion(selectedDirectory, options);
  if (!fromVersion) {
    return endMigration({
      success: false,
      message: `Unable to infer the version of turbo being used by ${directory}`,
    });
  }

  // step 2
  const toVersion = await getLatestVersion(options);
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
  const codemods = await getCodemodsForMigration({ fromVersion, toVersion });
  if (codemods.length === 0) {
    console.log(
      `No codemods required to migrate from ${fromVersion} to ${toVersion}`,
      os.EOL
    );
  }

  // step 4
  console.log(
    `Upgrading turbo from ${chalk.bold(fromVersion)} to ${chalk.bold(
      toVersion
    )}`,
    os.EOL
  );
  const results = codemods.map((codemod, idx) => {
    console.log(
      `(${idx + 1}/${codemods.length}) ${chalk.bold(
        `Running ${codemod.value}`
      )}`
    );

    const result = codemod.transformer({ root: selectedDirectory, options });
    Runner.logResults(result);
    return result;
  });

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
  const upgradeCommand = getTurboUpgradeCommand({
    directory: selectedDirectory,
    to: options.to,
  });

  if (!upgradeCommand) {
    return endMigration({
      success: false,
      message: "Unable to determine upgrade command",
    });
  }

  if (options.install) {
    if (options.dry) {
      console.log(`Upgrading turbo with ${chalk.bold(upgradeCommand)} ${chalk.dim('(dry run)')}`, os.EOL);
    } else {
      console.log(`Upgrading turbo with ${chalk.bold(upgradeCommand)}`, os.EOL);
      execSync(upgradeCommand, { cwd: selectedDirectory });
    }
  } else {
    console.log(`Upgrade turbo with ${chalk.bold(upgradeCommand)}`, os.EOL);
  }

  endMigration({ success: true });
}
