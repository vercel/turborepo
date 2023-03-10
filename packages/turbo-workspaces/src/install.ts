import execa from "execa";
import ora from "ora";
import { satisfies } from "semver";
import { ConvertError } from "./errors";
import { Logger } from "./logger";
import {
  PackageManager,
  PackageManagerDetails,
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
      default: true,
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
      default: true,
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
      default: true,
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

export function getPackageManagerMeta(packageManager: PackageManagerDetails) {
  const { version, name } = packageManager;

  let pm = PACKAGE_MANAGERS[name].find((manager) => {
    return manager.default;
  });

  if (version) {
    pm = PACKAGE_MANAGERS[name].find((manager) =>
      satisfies(version, manager.semver)
    );
  }

  return pm;
}

export default async function install(args: InstallArgs) {
  const { to, logger, options } = args;

  const installLogger = logger ?? new Logger(options);
  const packageManager = getPackageManagerMeta(to);

  if (!packageManager) {
    throw new ConvertError("Unsupported package manager version.");
  }

  installLogger.subStep(
    `running "${packageManager.command} ${packageManager.installArgs}"`
  );
  if (!options?.dry) {
    let spinner;
    if (options?.interactive) {
      spinner = ora({
        text: "Installing dependencies...",
        spinner: {
          frames: installLogger.installerFrames(),
        },
      }).start();
    }

    try {
      await execa(packageManager.command, packageManager.installArgs, {
        cwd: args.project.paths.root,
      });
      installLogger.subStep(`dependencies installed`);
    } catch (err) {
      installLogger.subStepFailure(`failed to install dependencies`);
      throw err;
    } finally {
      if (spinner) {
        spinner.stop();
      }
    }
  }
}
