import { cyan, bold, yellow } from "picocolors";
import checkForUpdate from "update-check";
import { logger } from "@turbo/utils";
import { getWorkspaceDetails } from "@turbo/workspaces";
import cliPkgJson from "../../package.json";

const update = checkForUpdate(cliPkgJson).catch(() => null);

export async function notifyUpdate(): Promise<void> {
  try {
    const res = await update;
    if (res?.latest) {
      const { packageManager } = await getWorkspaceDetails({
        root: process.cwd(),
      });

      let upgradeCommand = "npm i -g @turbo/codemod";
      if (packageManager === "yarn") {
        upgradeCommand = "yarn global add @turbo/codemod";
      } else if (packageManager === "pnpm") {
        upgradeCommand = "pnpm i -g @turbo/codemod";
      }

      logger.log();
      logger.log(
        yellow(bold("A new version of `@turbo/codemod` is available!"))
      );
      logger.log(`You can update by running: ${cyan(upgradeCommand)}`);
      logger.log();
    }
    process.exit();
  } catch (_) {
    // ignore error
  }
}
