import execa from "execa";
import ora from "ora";
import { satisfies } from "semver";
import { ConvertError } from "./errors";
import {
  PackageManager,
  PackageManagerInstallDetails,
  InstallArgs,
} from "./types";

export const PACKAGE_MANAGERS: Record<
  PackageManager,
  Array<PackageManagerInstallDetails>
> = {
  npm: [
    {
      name: "npm",
      template: "npm",
      command: "npm",
      installArgs: ["install"],
      version: "latest",
      executable: "npx",
      semver: "*",
    },
  ],
  pnpm: [
    {
      name: "pnpm6",
      template: "pnpm",
      command: "pnpm",
      installArgs: ["install"],
      version: "latest-6",
      executable: "pnpx",
      semver: "6.x",
    },
    {
      name: "pnpm",
      template: "pnpm",
      command: "pnpm",
      installArgs: ["install"],
      version: "latest",
      executable: "pnpm dlx",
      semver: ">=7",
    },
  ],
  yarn: [
    {
      name: "yarn",
      template: "yarn",
      command: "yarn",
      installArgs: ["install"],
      version: "1.x",
      executable: "npx",
      semver: "<2",
    },
    {
      name: "berry",
      template: "berry",
      command: "yarn",
      installArgs: ["install", "--no-immutable"],
      version: "stable",
      executable: "yarn dlx",
      semver: ">=2",
    },
  ],
};

async function install(args: InstallArgs) {
  const { to, logger, options } = args;
  let packageManager = PACKAGE_MANAGERS[to.name].find((manager) =>
    satisfies(to.version, manager.semver)
  );

  if (!packageManager) {
    throw new ConvertError("Unsupported package manager version.");
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
      await execa(packageManager.command, packageManager.installArgs, {
        cwd: args.project.paths.root,
      });
      spinner.stop();
      logger.subStep(`dependencies installed`);
    } catch (err) {
      spinner.stop();
      logger.subStepFailure(`failed to install dependencies`);
      throw err;
    }
  }
}

export default install;
