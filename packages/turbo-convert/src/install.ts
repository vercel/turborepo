import execa from "execa";
import chalk from "chalk";
import ora from "ora";
import { satisfies } from "semver";
import { PACKAGE_MANAGERS, InstallArgs } from "./types";

async function install(args: InstallArgs) {
  const { to, logger, options } = args;
  let packageManager = PACKAGE_MANAGERS[to.name].find((manager) =>
    satisfies(to.version, manager.semver)
  );

  if (!packageManager) {
    throw new Error("Unsupported package manager version.");
  }

  logger.subStep(
    `running "${packageManager.command} ${packageManager.installArgs}"`
  );
  if (!options?.dry) {
    const spinner = ora({
      text: "Installing dependencies...",
      spinner: {
        frames: logger.installerFrames(),
      },
    }).start();

    try {
      await execa(`${packageManager.command}`, packageManager.installArgs, {
        cwd: args.project.paths.root,
      });
      spinner.stop();
      logger.subStep(`dependencies installed`);
    } catch (error) {
      spinner.stop();
      logger.subStepFailure(`failed to install dependencies`);
      throw error;
    }
  }
}

export default install;
