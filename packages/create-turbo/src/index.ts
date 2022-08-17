#!/usr/bin/env node

import * as path from "path";
import execa from "execa";
import fse from "fs-extra";
import inquirer from "inquirer";
import ora from "ora";
import meow from "meow";
import lt from "semver/functions/lt";
import gradient from "gradient-string";
import checkForUpdate from "update-check";
import chalk from "chalk";
import cliPkgJson from "../package.json";
import { shouldUseYarn } from "./shouldUseYarn";
import { shouldUsePnpm, getNpxCommandOfPnpm } from "./shouldUsePnpm";
import { tryGitInit } from "./git";
import { PackageManager, PACKAGE_MANAGERS } from "./constants";
import { getPackageManagerVersion } from "./getPackageManagerVersion";

interface Answers {
  packageManager: PackageManager;
}

const turboGradient = gradient("#0099F7", "#F11712");

const help = `
  Usage:
    $ npx create-turbo [flags...] [<dir>]

  If <dir> is not provided up front you will be prompted for it.

  Flags:
    --use-npm           Explicitly tell the CLI to bootstrap the app using npm
    --use-pnpm          Explicitly tell the CLI to bootstrap the app using pnpm
    --use-yarn          Explicitly tell the CLI to bootstrap the app using yarn
    --no-install        Explicitly do not run the package manager's install command
    --help, -h          Show this help message
    --version, -v       Show the version of this script
`;

run()
  .then(notifyUpdate)
  .catch(async (reason) => {
    console.log();
    console.log("Aborting installation.");
    if (reason.command) {
      console.log(`  ${chalk.cyan(reason.command)} has failed.`);
    } else {
      console.log(chalk.red("Unexpected error. Please report it as a bug:"));
      console.log(reason);
    }
    console.log();

    await notifyUpdate();

    process.exit(1);
  });

async function run() {
  let { input, flags, showHelp, showVersion } = meow(help, {
    booleanDefault: undefined,
    flags: {
      help: { type: "boolean", default: false, alias: "h" },
      useNpm: { type: "boolean", default: false },
      usePnpm: { type: "boolean", default: false },
      useYarn: { type: "boolean", default: false },
      install: { type: "boolean", default: true },
      version: { type: "boolean", default: false, alias: "v" },
    },
  });

  if (flags.help) showHelp();
  if (flags.version) showVersion();

  // let anim = chalkAnimation.pulse(`\n>>> TURBOREPO\n`);
  console.log(chalk.bold(turboGradient(`\n>>> TURBOREPO\n`)));
  await new Promise((resolve) => setTimeout(resolve, 500));
  console.log(
    ">>> Welcome to Turborepo! Let's get you set up with a new codebase."
  );
  console.log();

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
              message: "Where would you like to create your turborepo?",
              default: "./my-turborepo",
            },
          ])
        ).dir
  );
  const projectName = path.basename(projectDir);

  const isYarnInstalled = shouldUseYarn();
  const isPnpmInstalled = shouldUsePnpm();
  let answers: Answers;
  if (flags.useNpm) {
    answers = { packageManager: PACKAGE_MANAGERS["npm"] };
  } else if (flags.usePnpm) {
    answers = { packageManager: PACKAGE_MANAGERS["pnpm"] };
  } else if (flags.useYarn) {
    answers = { packageManager: PACKAGE_MANAGERS["yarn"] };
  } else {
    answers = await inquirer.prompt<{
      packageManager: PackageManager;
    }>([
      {
        name: "packageManager",
        type: "list",
        message: "Which package manager do you want to use?",
        choices: [
          { name: "npm", value: "npm" },
          {
            name: "pnpm",
            value: "pnpm",
            disabled: !isPnpmInstalled && "not installed",
          },
          {
            name: "yarn",
            value: "yarn",
            disabled: !isYarnInstalled && "not installed",
          },
        ],
      },
    ]);
  }

  // Create the app directory
  let relativeProjectDir = path.relative(process.cwd(), projectDir);
  let projectDirIsCurrentDir = relativeProjectDir === "";
  if (!projectDirIsCurrentDir) {
    if (fse.existsSync(projectDir)) {
      console.log(
        `️🚨 Oops, "${relativeProjectDir}" already exists. Please try again with a different directory.`
      );
      process.exit(1);
    } else {
      await fse.mkdir(projectDir);
    }
  }

  // copy the shared template
  let sharedTemplate = path.resolve(__dirname, "../templates", `_shared_ts`);
  await fse.copy(sharedTemplate, projectDir);

  // copy the server template
  let serverTemplate = path.resolve(
    __dirname,
    "../templates",
    answers.packageManager.command
  );
  if (fse.existsSync(serverTemplate)) {
    await fse.copy(serverTemplate, projectDir, { overwrite: true });
  }

  // rename dotfiles
  await fse.move(
    path.join(projectDir, "gitignore"),
    path.join(projectDir, ".gitignore")
  );

  // merge package.jsons
  let sharedPkg = require(path.join(sharedTemplate, "package.json"));
  let projectPkg = require(path.join(projectDir, "package.json"));

  // add current versions of wildcard deps and merge
  ["dependencies", "devDependencies"].forEach((pkgKey) => {
    // merge dependencies, giving priority to the project deps
    sharedPkg[pkgKey] = {
      ...sharedPkg[pkgKey],
      ...projectPkg[pkgKey],
    };
  });

  sharedPkg.packageManager = `${
    answers.packageManager
  }@${getPackageManagerVersion(answers.packageManager)}`;
  sharedPkg.name = projectName;

  // write package.json
  await fse.writeFile(
    path.join(projectDir, "package.json"),
    JSON.stringify(sharedPkg, null, 2)
  );

  if (flags.install) {
    console.log();
    console.log(`>>> Creating a new turborepo with the following:`);
    console.log();
    console.log(` - ${chalk.bold("apps/web")}: Next.js with TypeScript`);
    console.log(` - ${chalk.bold("apps/docs")}: Next.js with TypeScript`);
    console.log(
      ` - ${chalk.bold("packages/ui")}: Shared React component library`
    );
    console.log(
      ` - ${chalk.bold(
        "packages/eslint-config-custom"
      )}: Shared configuration (ESLint)`
    );
    console.log(
      ` - ${chalk.bold(
        "packages/tsconfig"
      )}: Shared TypeScript \`tsconfig.json\``
    );
    console.log();

    const spinner = ora({
      text: "Installing dependencies...",
      spinner: {
        frames: ["   ", ">  ", ">> ", ">>>"],
      },
    }).start();

    let supportsRegistryArg = false;
    try {
      // yarn >= v2 only support specifying a registry via config (no cli param)
      supportsRegistryArg = lt(
        getPackageManagerVersion(answers.packageManager),
        "2.0.0"
      );
    } catch (err) {}

    const installArgs = ["install"];
    if (supportsRegistryArg) {
      // Using the official npm registry for installation could be very
      // slow for users in different regions (like China), so use the
      // user customized registry from the config instead
      const npmRegistry = await getNpmRegistry(answers.packageManager);
      installArgs.push(`--registry=${npmRegistry}`);
    }

    await execa(`${answers.packageManager}`, installArgs, {
      stdio: "ignore",
      cwd: projectDir,
    });
    spinner.stop();
  } else {
    console.log();
    console.log(`>>> Bootstrapped a new turborepo with the following:`);
    console.log();
    console.log(` - ${chalk.bold("apps/web")}: Next.js with TypeScript`);
    console.log(` - ${chalk.bold("apps/docs")}: Next.js with TypeScript`);
    console.log(
      ` - ${chalk.bold("packages/ui")}: Shared React component library`
    );
    console.log(
      ` - ${chalk.bold("packages/config")}: Shared configuration (ESLint)`
    );
    console.log(
      ` - ${chalk.bold(
        "packages/tsconfig"
      )}: Shared TypeScript \`tsconfig.json\``
    );
    console.log();
  }

  process.chdir(projectDir);
  tryGitInit(relativeProjectDir);
  if (projectDirIsCurrentDir) {
    console.log(
      `${chalk.bold(
        turboGradient(">>> Success!")
      )} Your new Turborepo is ready. `
    );
    console.log("Inside this directory, you can run several commands:");
  } else {
    console.log(
      `${chalk.bold(
        turboGradient(">>> Success!")
      )} Created a new Turborepo at "${relativeProjectDir}". `
    );
    console.log("Inside that directory, you can run several commands:");
  }

  console.log();
  console.log(chalk.cyan(`  ${answers.packageManager} run build`));
  console.log(`     Build all apps and packages`);
  console.log();
  console.log(chalk.cyan(`  ${answers.packageManager} run dev`));
  console.log(`     Develop all apps and packages`);
  console.log();
  console.log(`Turborepo will cache locally by default. For an additional`);
  console.log(`speed boost, enable Remote Caching with Vercel by`);
  console.log(`entering the following command:`);
  console.log();
  console.log(
    chalk.cyan(`  ${getNpxCommand(answers.packageManager)} turbo login`)
  );
  console.log();
  console.log(`We suggest that you begin by typing:`);
  console.log();
  if (!projectDirIsCurrentDir) {
    console.log(`  ${chalk.cyan("cd")} ${relativeProjectDir}`);
  }
  console.log(
    chalk.cyan(`  ${getNpxCommand(answers.packageManager)} turbo login`)
  );
  console.log();
}

async function getNpmRegistry(pkgManager: PackageManager): Promise<string> {
  try {
    // npm/pnpm/yarn share the same CLI configuration commands
    const { stdout: registry } = await execa(pkgManager.command, [
      "config",
      "get",
      "registry",
    ]);
    return registry;
  } catch (error) {
    return "";
  }
}

const update = checkForUpdate(cliPkgJson).catch(() => null);

async function notifyUpdate(): Promise<void> {
  try {
    const res = await update;
    if (res?.latest) {
      const isYarn = shouldUseYarn();

      console.log();
      console.log(
        chalk.yellow.bold("A new version of `create-turbo` is available!")
      );
      console.log(
        "You can update by running: " +
          chalk.cyan(
            isYarn ? "yarn global add create-turbo" : "npm i -g create-turbo"
          )
      );
      console.log();
    }
    process.exit();
  } catch {
    // ignore error
  }
}

function getNpxCommand(pkgManager: PackageManager): string {
  if (pkgManager.command === "pnpm") {
    return getNpxCommandOfPnpm();
  } else {
    return "npx";
  }
}
