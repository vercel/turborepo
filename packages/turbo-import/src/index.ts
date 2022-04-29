#!/usr/bin/env node

import chalk from "chalk";
import globby from "globby";
import inquirer from "inquirer";
import meow from "meow";
import fs from "fs-extra";
import path from "path";
import checkForUpdate from "update-check";
import cliPkgJson from "../package.json";
import { getWorkspaceImplementation } from "./getWorkspaceImplementation";
import { checkGitStatus } from "./git";
import { DetectorFilesystem, detectFramework } from "@vercel/build-utils";
import { frameworks } from "@vercel/frameworks";

const frameworkAddons = [
  {
    slug: "remix",
    async getOutputDirs(cwd: string) {
      try {
        const config = require(path.join(cwd, "remix.config.js"));
        let result = [];
        result.push(config.assetsBuildDirectory ?? "public/build");
        result.push(config.serverBuildDirectory ?? "build");
      } catch (error) {
        return ["public/build", "build"];
      }
    },
  },
];

class LocalDetectorFilesystem extends DetectorFilesystem {
  constructor() {
    super();
  }
  protected async _hasPath(name: string): Promise<boolean> {
    const res = fs.existsSync(name);
    return res;
  }
  protected async _readFile(name: string): Promise<Buffer> {
    return await fs.readFile(name);
  }
  protected async _isFile(name: string): Promise<boolean> {
    try {
      const info = await fs.lstat(name);
      return info.isDirectory() === false;
    } catch (error) {
      return false;
    }
  }
}

const help = `
  Usage:
    $ npx @turbo/import <path> <...options>

  If <path> is not provided up front you will be prompted for it.

  Options:    
    --force             Bypass Git safety checks and forcibly run codemods
    --dry               Dry run (no changes are made to files)
    --print             Print transformed files to your terminal
    --help, -h          Show this help message
    --version, -v       Show the version of this script
`;

run()
  // .then(notifyUpdate)
  .catch(async (reason) => {
    console.log("Aborting installation.");
    if (reason.command) {
      console.log(`  ${chalk.cyan(reason.command)} has failed.`);
    } else {
      console.log(chalk.red("Unexpected error. Please report it as a bug:"));
      console.log(reason);
    }
    console.log("fcuk");

    await notifyUpdate();

    process.exit(1);
  });

async function run() {
  let cli = meow(help, {
    booleanDefault: undefined,
    flags: {
      help: { type: "boolean", default: false, alias: "h" },
      force: { type: "boolean", default: false },
      dry: { type: "boolean", default: false },
      print: { type: "boolean", default: false },
      version: { type: "boolean", default: false, alias: "v" },
    },
    description: "Codemods for updating Turborepo codebases.",
  });

  if (cli.flags.help) cli.showHelp();
  if (cli.flags.version) cli.showVersion();
  const DetectFS = new LocalDetectorFilesystem();
  const framework = await detectFramework({
    fs: DetectFS,
    frameworkList: frameworks,
  });
  console.log(framework);
  const frameworkEntry = frameworks.find((f) => f.slug === framework);
  if (!frameworkEntry) {
    process.exit(1);
  }

  let outputDir = await frameworkEntry?.getFsOutputDir();
  let outputs: string[] = [];
  if (outputDir == null) {
    const potentialOutputs = await frameworkAddons
      .find((addon) => addon.slug === framework)
      ?.getOutputDirs(cli.input[0] || process.cwd());
    outputs = potentialOutputs?.map((o) => o + "/**") ?? [
      "build/**",
      "dist/**",
    ];
  } else {
    outputs = [outputDir + "/**"];
  }

  const turboJson = {
    pipeline: {
      build: {
        dependsOn: ["^build"],
        outputs: outputs,
      },
      lint: {
        dependsOn: [],
        outputs: [],
      },
      dev: {
        cache: false,
      },
      test: {
        dependsOn: ["^build"],
        outputs: [],
      },
    },
  };

  // TODO: @jaredpalmer
  // 1. move root files to folder called "app" aside from:
  //   -  root lockfile
  //   - .yarn to a folder
  //   - .gitignore
  //   - .github
  //   - .circleci
  // 2. Add a root package.json
  //      - Set a "version"
  //      - Set "private" to true
  //      - Add a "scripts" object with "build" "test" and "dev"
  //      - Add turbo as devDependency
  //      - If yarn or npm, set `workspaces` to inlcude the app folder.
  // 2a. If pnpm, create pnpm-workspace.yaml
  // 3. Write turbo.json
  // 4. Re-install node_modules with correct package manager

  // check git status
  // if (!cli.flags.dry) {
  //   checkGitStatus(cli.flags.force);
  // }

  // const answers = await inquirer.prompt([
  //   {
  //     type: "input",
  //     name: "files",
  //     message: "On which directory should the codemods be applied?",
  //     when: !cli.input[1],
  //     default: ".",
  //     // validate: () =>
  //     filter: (files) => files.trim(),
  //   },
  // ]);

  // const { files, transformer } = answers;

  // const filesBeforeExpansion = cli.input[1] || files;
  // const filesExpanded = expandFilePathsIfNeeded([filesBeforeExpansion]);

  // const selectedTransformer = cli.input[0] || transformer;

  // if (!filesExpanded.length) {
  //   console.log(`No files found matching ${filesBeforeExpansion.join(" ")}`);
  //   return null;
  // }

  return 0;
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

function expandFilePathsIfNeeded(filesBeforeExpansion: string[]) {
  const shouldExpandFiles = filesBeforeExpansion.some((file) =>
    file.includes("*")
  );
  return shouldExpandFiles
    ? globby.sync(filesBeforeExpansion)
    : filesBeforeExpansion;
}
