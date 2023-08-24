import chalk from "chalk";
import checkForUpdate from "update-check";
import cliPkgJson from "../../package.json";

const update = checkForUpdate(cliPkgJson).catch(() => null);

export async function notifyUpdate(): Promise<void> {
  try {
    const res = await update;
    if (res?.latest) {
      // eslint-disable-next-line no-console
      console.log();
      // eslint-disable-next-line no-console
      console.log(
        chalk.yellow.bold("A new version of `create-turbo` is available!")
      );
      // eslint-disable-next-line no-console
      console.log();
    }
    process.exit();
  } catch (_) {
    // ignore error
  }
}
