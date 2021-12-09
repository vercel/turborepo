#!/usr/bin/env node

import * as path from "path";
import execa from "execa";
import fse from "fs-extra";
import inquirer from "inquirer";
import ora from "ora";
import meow from "meow";
import gradient from "gradient-string";
import checkForUpdate from "update-check";
import chalk from "chalk";
import cliPkgJson from "../package.json";
import { shouldUseYarn } from "./shouldUseYarn";
import { tryGitInit } from "./git";

const turboGradient = gradient("#0099F7", "#F11712");
const help = `
  Usage:
    $ npx create-turbo [flags...] [<dir>]

  If <dir> is not provided up front you will be prompted for it.

  Flags:
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
    flags: {
      help: { type: "boolean", default: false, alias: "h" },
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

  let answers = await inquirer.prompt<{
    packageManager: "yarn" | "npm";
    install: boolean;
  }>([
    {
      name: "packageManager",
      type: "list",
      message: "Which package manager do you want to use?",
      choices: [
        { name: "Yarn", value: "yarn" },
        { name: "NPM", value: "npm" },
        // { name: "PNPM", value: "pnpm" },
      ],
    },
    {
      name: "install",
      type: "confirm",
      message: function (answers) {
        return `Do you want me to run \`${answers.packageManager} install\`?`;
      },
      default: true,
    },
  ]);

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
    answers.packageManager
  );
  if (fse.existsSync(serverTemplate)) {
    await fse.copy(serverTemplate, projectDir, { overwrite: true });
  }

  // let serverLangTemplate = path.resolve(
  //   __dirname,
  //   "templates",
  //   `${answers.packageManager}_ts`
  // );
  // if (fse.existsSync(serverLangTemplate)) {
  //   await fse.copy(serverLangTemplate, projectDir, { overwrite: true });
  // }

  // rename dotfiles
  await fse.move(
    path.join(projectDir, "gitignore"),
    path.join(projectDir, ".gitignore")
  );

  // merge package.jsons
  let appPkg = require(path.join(sharedTemplate, "package.json"));

  // add current versions of remix deps
  // ["dependencies", "devDependencies"].forEach((pkgKey) => {
  //   for (let key in appPkg[pkgKey]) {
  //     if (appPkg[pkgKey][key] === "*") {
  //       appPkg[pkgKey][key] = `latest`;
  //     }
  //   }
  // });

  // write package.json
  await fse.writeFile(
    path.join(projectDir, "package.json"),
    JSON.stringify(appPkg, null, 2)
  );

  if (answers.install) {
    console.log();
    console.log(`>>> Bootstrapping a new turborepo with the following:`);
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

    const spinner = ora({
      text: "Installing dependencies...",
      spinner: {
        frames: ["   ", ">  ", ">> ", ">>>"],
      },
    }).start();
    await execa(`${answers.packageManager}`, [`install`], {
      stdio: "ignore",
      cwd: projectDir,
    });
    spinner.stop();
  }

  process.chdir(relativeProjectDir);
  tryGitInit(relativeProjectDir);

  if (projectDirIsCurrentDir) {
    console;
    console.log(
      `${chalk.bold(
        turboGradient(">>> Success!")
      )} Check the README for development and deploy instructions!`
    );
  } else {
    console.log(
      `${chalk.bold(
        turboGradient(">>> Success!")
      )} Your new Turborepo is ready. `
    );
    console.log();
    console.log(`To build all apps and packages, run the following:`);
    console.log();
    console.log(`  cd ${relativeProjectDir}`);
    console.log(`  ${answers.packageManager} run build`);
    console.log();
    console.log(`To develop all apps and packages, run the following:`);
    console.log();
    console.log(`  cd ${relativeProjectDir}`);
    console.log(`  ${answers.packageManager} run dev`);
    console.log();
    console.log(`Turborepo will cache locally by default. For an additional`);
    console.log(`speed boost, enable Remote Caching (beta) with Vercel by`);
    console.log(`entering the following commands:`);
    console.log();
    console.log(`  cd ${relativeProjectDir}`);
    console.log(`  npx turbo login`);
    console.log();
    console.log(
      `For more info, checkout the README in ${chalk.bold(relativeProjectDir)}`
    );
    console.log(
      `as well as the official Turborepo docs ${chalk.underline(
        "https://turborepo.org"
      )}`
    );
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
