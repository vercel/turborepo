import picocolors from "picocolors";
import checkForUpdate from "update-check";
import * as logger from "./logger";
import type { ExitCode } from "./types";

interface PackageInfo {
  name: string;
  version: string;
}

interface NotifyUpdateOptions {
  /** The package to check for updates */
  packageInfo: PackageInfo;
  /** Optional upgrade command to display (string or async function that returns a string) */
  upgradeCommand?: string | (() => Promise<string | undefined>);
}

/**
 * Creates a notifyUpdate function for a CLI package.
 * This should be called at module load time to start the update check early.
 */
export function createNotifyUpdate(options: NotifyUpdateOptions) {
  const { packageInfo, upgradeCommand } = options;
  const update = checkForUpdate(packageInfo).catch(() => null);

  return async function notifyUpdate(exitCode: ExitCode = 0): Promise<void> {
    try {
      const res = await update;
      if (res?.latest) {
        logger.log();
        logger.log(
          picocolors.yellow(
            picocolors.bold(
              `A new version of \`${packageInfo.name}\` is available!`
            )
          )
        );
        const command =
          typeof upgradeCommand === "function"
            ? await upgradeCommand()
            : upgradeCommand;
        if (command) {
          logger.log(`You can update by running: ${picocolors.cyan(command)}`);
        }
        logger.log();
      }
      process.exit(exitCode);
    } catch (error) {
      if (process.env.DEBUG) {
        logger.error("Update check failed:", error);
      }
      process.exit(exitCode);
    }
  };
}
