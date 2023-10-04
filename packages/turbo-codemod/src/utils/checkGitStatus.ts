import chalk from "chalk";
import isGitClean from "is-git-clean";
import { logger } from "@turbo/utils";

export function checkGitStatus({
  directory,
  force,
}: {
  directory?: string;
  force: boolean;
}) {
  let clean = false;
  let errorMessage = "Unable to determine if git directory is clean";
  try {
    clean = isGitClean.sync(directory || process.cwd());
    errorMessage = "Git directory is not clean";
  } catch (err: unknown) {
    const errWithDetails = err as { stderr?: string };
    if (errWithDetails.stderr?.includes("not a git repository")) {
      clean = true;
    }
  }

  if (!clean) {
    if (force) {
      logger.log(
        `${chalk.yellow("WARNING")}: ${errorMessage}. Forcibly continuing...`
      );
    } else {
      logger.log("Thank you for using @turbo/codemod!");
      logger.log(
        chalk.yellow(
          "\nBut before we continue, please stash or commit your git changes."
        )
      );
      logger.log(
        "\nYou may use the --force flag to override this safety check."
      );
      process.exit(1);
    }
  }
}
