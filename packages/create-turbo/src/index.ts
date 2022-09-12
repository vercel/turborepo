#!/usr/bin/env node

import * as path from "path";
import execa from "execa";
import fs from "fs";
import inquirer from "inquirer";
import ora from "ora";
import meow from "meow";
import { satisfies } from "semver";
import gradient from "gradient-string";
import checkForUpdate from "update-check";
import chalk from "chalk";
import cliPkgJson from "../package.json";
import { shouldUseYarn } from "./shouldUseYarn";
import { shouldUsePnpm } from "./shouldUsePnpm";
import { tryGitInit } from "./git";
import { CommandName, PACKAGE_MANAGERS } from "./constants";
import { getPackageManagerVersion } from "./getPackageManagerVersion";

interface Answers {
  packageManager: CommandName;
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
    answers = { packageManager: "npm" };
  } else if (flags.usePnpm) {
    answers = { packageManager: "pnpm" };
  } else if (flags.useYarn) {
    answers = { packageManager: "yarn" };
  } else {
    answers = await inquirer.prompt<{
      packageManager: CommandName;
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
    if (fs.existsSync(projectDir) && fs.readdirSync(projectDir).length !== 0) {
      console.log(
        `️🚨 Oops, "${relativeProjectDir}" already exists. Please try again with a different directory.`
      );
      process.exit(1);
    } else {
      fs.mkdirSync(projectDir, { recursive: true });
    }
  }

  // copy the shared template
  let sharedTemplate = path.resolve(__dirname, "../templates", `_shared_ts`);
  fs.cpSync(sharedTemplate, projectDir, { recursive: true });

  let packageManagerVersion = getPackageManagerVersion(answers.packageManager);
  let packageManagerConfigs = PACKAGE_MANAGERS[answers.packageManager];
  let packageManager = packageManagerConfigs.find((packageManager) =>
    satisfies(packageManagerVersion, packageManager.semver)
  );

  if (!packageManager) {
    throw new Error("Unsupported package manager version.");
  }

  // copy the per-package-manager template
  let packageManagerTemplate = path.resolve(
    __dirname,
    "../templates",
    packageManager.template
  );
  if (fs.existsSync(packageManagerTemplate)) {
    fs.cpSync(packageManagerTemplate, projectDir, {
      recursive: true,
      force: true,
    });
  }

  // rename dotfiles
  fs.renameSync(
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

  sharedPkg.packageManager = `${packageManager.command}@${packageManagerVersion}`;
  sharedPkg.name = projectName;

  // write package.json
  fs.writeFileSync(
    path.join(projectDir, "package.json"),
    JSON.stringify(sharedPkg, null, 2)
  );

  console.log();
  console.log(`>>> Created a new turborepo with the following:`);
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
    ` - ${chalk.bold("packages/tsconfig")}: Shared TypeScript \`tsconfig.json\``
  );
  console.log();

  if (flags.install) {
    const spinner = ora({
      text: "Installing dependencies...",
      spinner: {
        frames: ["   ", ">  ", ">> ", ">>>"],
      },
    }).start();

    await execa(`${packageManager.command}`, packageManager.installArgs, {
      stdio: "ignore",
      cwd: projectDir,
    });
    spinner.stop();
  }

  process.chdir(projectDir);
  tryGitInit(relativeProjectDir);
  if (projectDirIsCurrentDir) {
    console.log(
      `${chalk.bold(
        turboGradient(">>> Success!")
      )} Your new Turborepo is ready.`
    );
    console.log("Inside this directory, you can run several commands:");
  } else {
    console.log(
      `${chalk.bold(
        turboGradient(">>> Success!")
      )} Created a new Turborepo at "${relativeProjectDir}".`
    );
    console.log("Inside that directory, you can run several commands:");
  }

  console.log();
  console.log(chalk.cyan(`  ${packageManager.command} run build`));
  console.log(`     Build all apps and packages`);
  console.log();
  console.log(chalk.cyan(`  ${packageManager.command} run dev`));
  console.log(`     Develop all apps and packages`);
  console.log();
  console.log(`Turborepo will cache locally by default. For an additional`);
  console.log(`speed boost, enable Remote Caching with Vercel by`);
  console.log(`entering the following command:`);
  console.log();
  console.log(chalk.cyan(`  ${packageManager.executable} turbo login`));
  console.log();
  console.log(`We suggest that you begin by typing:`);
  console.log();
  if (!projectDirIsCurrentDir) {
    console.log(`  ${chalk.cyan("cd")} ${relativeProjectDir}`);
  }
  console.log(chalk.cyan(`  ${packageManager.executable} turbo login`));
  console.log();
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
