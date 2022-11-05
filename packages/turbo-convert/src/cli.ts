#!/usr/bin/env node

import inquirer from "inquirer";
import path from "path";
import meow from "meow";

import {
  isYarnAvailable,
  isPnpmAvailable,
  isNpmAvailable,
  PackageManagerAvailable,
} from "turbo-utils";
import getWorkspaceDetails from "./getWorkspaceDetails";
import convert from "./convert";
import { PackageManagers } from "./types";
import { Logger } from "./logger";

// find all available package managers
const availablePackageManagers: Record<
  PackageManagers,
  PackageManagerAvailable
> = {
  yarn: isYarnAvailable(),
  pnpm: isPnpmAvailable(),
  npm: isNpmAvailable(),
};

function isPackageManagerDisabled({
  packageManager,
  currentWorkspaceManger,
}: {
  packageManager: PackageManagers;
  currentWorkspaceManger: PackageManagers;
}) {
  if (currentWorkspaceManger === packageManager) {
    return "already in use";
  }

  if (!availablePackageManagers[packageManager].available) {
    return "not installed";
  }

  return false;
}

interface Answers {
  packageManager: PackageManagers;
}

const help = `
  Usage:
    $ npx turbo-convert [flags...] [<dir>]

  If <dir> is not provided, you will be prompted for it.

  Flags:
    --npm           Switch to npm workspaces
    --pnpm          Switch to pnpm workspaces
    --yarn          Switch to yarn workspaces
    --install       Convert lock files and install dependencies
    --dry           Run without making any modifications
    --summary       Show a summary of the workspace at <dir>, including the detected package
                    manager, all workspaces, and their locations
    --help, -h      Show this help message
    --version, -v   Show the version of this script
`;

run().catch((err: Error) => {
  if (err?.cause !== "expected") {
    console.log("Unexpected error - aborting");
    console.error(err.message || err);
    process.exit(1);
  }
  console.log();
  console.error(err.message || err);
  process.exit(0);
});

async function run() {
  let { input, flags, showHelp, showVersion } = meow(help, {
    booleanDefault: undefined,
    flags: {
      npm: { type: "boolean", default: false },
      pnpm: { type: "boolean", default: false },
      yarn: { type: "boolean", default: false },
      install: { type: "boolean" },
      dry: { type: "boolean", default: false },
      summary: { type: "boolean", default: false },
      help: { type: "boolean", default: false, alias: "h" },
      version: { type: "boolean", default: false, alias: "v" },
    },
  });

  if (flags.help) showHelp();
  if (flags.version) showVersion();
  const logger = new Logger({ dry: flags.dry, interactive: true });

  logger.hero();
  await new Promise((resolve) => setTimeout(resolve, 500));
  logger.header("Welcome, let's migrate your project.");
  // Figure out the app directory
  let projectDir = path.resolve(
    process.cwd(),
    input.length > 0
      ? input[0]
      : (
          await inquirer.prompt<{ dir: string }>([
            {
              type: "input",
              name: "dir",
              message: "Where is the root of your monorepo?",
              default: ".",
            },
          ])
        ).dir
  );

  let shouldInstall = flags.install;
  if (shouldInstall === undefined) {
    const installResponse = await inquirer.prompt<{ install: string }>([
      {
        type: "list",
        name: "install",
        message: "Should we automatically install after conversion?",
        default: "yes",
        choices: ["yes", "no"],
      },
    ]);
    shouldInstall = installResponse.install === "yes";
  }

  // if it's absolute, don't worry about it
  const workspaceRoot = path.isAbsolute(projectDir)
    ? projectDir
    : path.relative(process.cwd(), projectDir);
  const project = getWorkspaceDetails({ workspaceRoot });

  // log workspace summary
  if (flags.summary) {
    logger.workspaceSummary({ project, workspaceRoot });
  }

  if (flags[project.packageManager]) {
    throw new Error("You are already using this package manager", {
      cause: "expected",
    });
  }

  let answers: Answers;
  if (flags.npm) {
    answers = { packageManager: "npm" };
  } else if (flags.pnpm) {
    answers = { packageManager: "pnpm" };
  } else if (flags.yarn) {
    answers = { packageManager: "yarn" };
  } else {
    answers = await inquirer.prompt<{
      packageManager: PackageManagers;
    }>([
      {
        name: "packageManager",
        type: "list",
        message: `Convert from ${project.packageManager} workspaces to:`,
        choices: ["npm", "pnpm", "yarn"].map((p) => ({
          name: `${p} workspaces`,
          value: p,
          disabled: isPackageManagerDisabled({
            packageManager: p as PackageManagers,
            currentWorkspaceManger: project.packageManager,
          }),
        })),
      },
    ]);
  }

  await convert({
    project,
    to: {
      name: answers.packageManager,
      version: availablePackageManagers[answers.packageManager]
        .version as string,
    },
    logger,
    options: { dry: flags.dry, install: shouldInstall },
  });
}
