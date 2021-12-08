import * as path from "path";
import { execSync } from "child_process";
import fse from "fs-extra";
import inquirer from "inquirer";
import meow from "meow";
import gradient from "gradient-string";
import kleur from "kleur";
import cliPkgJson from "../package.json";

const turboGradient = gradient("#0099F7", "#F11712");
const help = `
  Usage:
    $ npx create-turborepo [flags...] [<dir>]

  If <dir> is not provided up front you will be prompted for it.

  Flags:
    --help, -h          Show this help message
    --version, -v       Show the version of this script
`;

run().then(
  () => {
    process.exit(0);
  },
  (error) => {
    console.error(error);
    process.exit(1);
  }
);

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
  console.log(kleur.bold(turboGradient(`\n>>> TURBOREPO\n`)));
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
  ["dependencies", "devDependencies"].forEach((pkgKey) => {
    for (let key in appPkg[pkgKey]) {
      if (appPkg[pkgKey][key] === "*") {
        appPkg[pkgKey][key] = `^${cliPkgJson.version}`;
      }
    }
  });

  // write package.json
  await fse.writeFile(
    path.join(projectDir, "package.json"),
    JSON.stringify(appPkg, null, 2)
  );

  if (answers.install) {
    execSync(`${answers.packageManager} install`, {
      stdio: "inherit",
      cwd: projectDir,
    });
  }

  if (projectDirIsCurrentDir) {
    console;
    console.log(
      `${kleur.bold(
        turboGradient(">>> Success!")
      )} Check the README for development and deploy instructions!`
    );
  } else {
    console.log(
      `${kleur.bold(
        turboGradient(">>> Success!")
      )} \`cd\` into "${path.relative(
        process.cwd(),
        projectDir
      )}" and check the README for development and deploy instructions!`
    );
  }
}
