#!/usr/bin/env node

import chalk from "chalk";
import globby from "globby";
import inquirer from "inquirer";
import meow from "meow";
import checkForUpdate from "update-check";
import cliPkgJson from "../package.json";
import { getWorkspaceImplementation } from "./getWorkspaceImplementation";
import { checkGitStatus } from "./git";
import { runTransform } from "./runTransform";

const help = `
  Usage:
    $ npx @turbo/codemod <transform> <path> <...options>

  If <path> is not provided up front you will be prompted for it.

  Options:
    --force             Bypass Git safety checks and forcibly run codemods
    --dry               Dry run (no changes are made to files)
    --print             Print transformed files to your terminal
    --list              List all codemods
    --help, -h          Show this help message
    --version, -v       Show the version of this script
`;

const TRANSFORMER_INQUIRER_CHOICES = [
  {
    name: "add-package-manager: Set the `packageManager` key in root `package.json` file",
    value: "add-package-manager",
    introducedIn: "1.1.0",
  },
  {
    name: 'create-turbo-config: Create the `turbo.json` file from an existing "turbo" key in `package.json`',
    value: "create-turbo-config",
    introducedIn: "1.1.0",
  },
  {
    name: 'migrate-env-var-dependencies: Migrate environment variable dependencies from "dependsOn" to "env" in `turbo.json`',
    value: "migrate-env-var-dependencies",
    introducedIn: "1.5.0",
  },
  {
    name: 'set-default-outputs: Add the "outputs" key with defaults where it is missing in `turbo.json`',
    value: "set-default-outputs",
    introducedIn: "1.7.0",
  },
];

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
  let cli = meow(help, {
    booleanDefault: undefined,
    flags: {
      help: { type: "boolean", default: false, alias: "h" },
      list: { type: "boolean", default: false },
      force: { type: "boolean", default: false },
      dry: { type: "boolean", default: false },
      print: { type: "boolean", default: false },
      version: { type: "boolean", default: false, alias: "v" },
    },
    description: "Codemods for updating Turborepo codebases.",
  });

  if (cli.flags.help) cli.showHelp();
  if (cli.flags.version) cli.showVersion();
  if (cli.flags.list) listTransforms();

  // check git status
  if (!cli.flags.dry) {
    checkGitStatus(cli.flags.force);
  }

  if (
    cli.input[0] &&
    !TRANSFORMER_INQUIRER_CHOICES.find((x) => x.value === cli.input[0])
  ) {
    console.error("Invalid transform choice, pick one of:");
    console.error(
      TRANSFORMER_INQUIRER_CHOICES.map((x) => "- " + x.value).join("\n")
    );
    process.exit(1);
  }
  const answers = await inquirer.prompt([
    {
      type: "input",
      name: "files",
      message: "On which directory should the codemods be applied?",
      when: !cli.input[1],
      default: ".",
      // validate: () =>
      filter: (files) => files.trim(),
    },
    {
      type: "list",
      name: "transformer",
      message: "Which transform would you like to apply?",
      when: !cli.input[0],
      pageSize: TRANSFORMER_INQUIRER_CHOICES.length,
      choices: TRANSFORMER_INQUIRER_CHOICES,
    },
  ]);

  const { files, transformer } = answers;

  const filesBeforeExpansion = cli.input[1] || files;
  const filesExpanded = expandFilePathsIfNeeded([filesBeforeExpansion]);

  const selectedTransformer = cli.input[0] || transformer;

  if (!filesExpanded.length) {
    console.log(`No files found matching ${filesBeforeExpansion.join(" ")}`);
    return null;
  }

  return runTransform({
    files: filesExpanded,
    flags: cli.flags,
    transformer: selectedTransformer,
  });
}

const update = checkForUpdate(cliPkgJson).catch(() => null);

async function notifyUpdate(): Promise<void> {
  try {
    const res = await update;
    if (res?.latest) {
      const ws = getWorkspaceImplementation(process.cwd());

      console.log();
      console.log(
        chalk.yellow.bold("A new version of `@turbo/codemod` is available!")
      );
      console.log(
        "You can update by running: " +
          chalk.cyan(
            ws === "yarn"
              ? "yarn global add @turbo/codemod"
              : ws === "pnpm"
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

function listTransforms(): void {
  console.log(
    TRANSFORMER_INQUIRER_CHOICES.map((x) => `- ${chalk.cyan(x.value)}`).join(
      "\n"
    )
  );
  process.exit(0);
}

function expandFilePathsIfNeeded(filesBeforeExpansion: string[]) {
  const shouldExpandFiles = filesBeforeExpansion.some((file) =>
    file.includes("*")
  );
  return shouldExpandFiles
    ? globby.sync(filesBeforeExpansion)
    : filesBeforeExpansion;
}
