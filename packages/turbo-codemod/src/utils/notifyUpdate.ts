import chalk from "chalk";
import checkForUpdate from "update-check";

import cliPkgJson from "../../package.json";
import { getWorkspaceDetails } from "@turbo/workspaces";

const update = checkForUpdate(cliPkgJson).catch(() => null);

export default async function notifyUpdate(): Promise<void> {
  try {
    const res = await update;
    if (res?.latest) {
      const project = await getWorkspaceDetails({ root: process.cwd() });

      const { packageManager } = project || {};

      console.log();
      console.log(
        chalk.yellow.bold("A new version of `@turbo/codemod` is available!")
      );
      console.log(
        "You can update by running: " +
          chalk.cyan(
            packageManager === "yarn"
              ? "yarn global add @turbo/codemod"
              : packageManager === "pnpm"
              ? "pnpm i -g @turbo/codemod"
              : "npm i -g @turbo/codemod"
          )
      );
      console.log();
    }
    process.exit();
  } catch (_e: any) {
    // ignore error
  }
}
